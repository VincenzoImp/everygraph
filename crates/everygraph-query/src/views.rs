// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Filtered views (#93; ADR 0010; `docs/spec/graph-semantics.md` § 6).
//!
//! A [`ViewSpec`] is a definition: predicates on node, pair and event columns, and node
//! sets. Making one reads nothing. [`ViewSpec::build`] evaluates the predicates into
//! bitsets, in parallel, and makes the [`View`] in one pass over the parent's lists,
//! which counts its nodes and edges. The view runs as a mask until
//! [`Filtered::hold`] materializes it by the planner's rule (`planner::view_form`).
//!
//! What a view keeps (`design/DECISIONS.md`, "Nodes in a view"):
//!
//! - a node of the parent when every node predicate holds and the node is in every node
//!   set;
//! - an entry of the parent (a pair) when both its ends are kept, every pair predicate
//!   holds, and, with event predicates, at least one of the pair's events passes them
//!   all;
//! - with a pair or event predicate, only the kept nodes that still have a kept entry,
//!   in either direction. Without one, every kept node: the induced subgraph.
//!
//! Predicates combine by "and", and [`ViewSpec::and`] combines two definitions. A
//! comparison with a null or a NaN is false, so such a row is never kept
//! (`graph-semantics.md` § 2.6). An integer column compares exactly with an integer or
//! a float. A time window is [`Test::Between`] on a time column: both ends are inside.
//!
//! The kernels' per-entry columns of a view are the parent's, gathered in the view's
//! entry order ([`Filtered::pair_values`]). On a view with event predicates, a pair's
//! multiplicity and sums are those of the events it keeps ([`Filtered::multiplicity`],
//! [`Filtered::event_sums`]; ADR 0028).

use std::cmp::Ordering::{self, Equal, Greater, Less};
use std::num::NonZeroUsize;
use std::ops::Range;

use everygraph_core::{ExactSum, Memory, SegVec, par};
use everygraph_kernels::edge_map::{Error, Settings, Source, View};
use everygraph_kernels::planner::{self, ViewForm};

/// Words of a bitset per task: 65,536 rows.
const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 10).expect("non-zero");

const SEGMENT: Error = Error::Invalid("a view bitset longer than one segment (#m3-15)");

/// The values of a column, one per row.
#[derive(Clone, Copy, Debug)]
pub enum Values<'a> {
    /// `float64`.
    F64(&'a [f64]),
    /// `float32`.
    F32(&'a [f32]),
    /// `int64`, and timestamps.
    I64(&'a [i64]),
    /// `int32`.
    I32(&'a [i32]),
    /// `uint64`.
    U64(&'a [u64]),
    /// `uint32`: labels, such as communities, and dictionary codes.
    U32(&'a [u32]),
}

impl Values<'_> {
    fn len(&self) -> usize {
        match self {
            Self::F64(v) => v.len(),
            Self::F32(v) => v.len(),
            Self::I64(v) => v.len(),
            Self::I32(v) => v.len(),
            Self::U64(v) => v.len(),
            Self::U32(v) => v.len(),
        }
    }
}

/// A column: its values, and its validity bitmap as Arrow stores it (bit `i`, least
/// significant first, set when row `i` is not null).
#[derive(Clone, Copy, Debug)]
pub struct Column<'a> {
    /// The values.
    pub values: Values<'a>,
    /// The validity bitmap; `None`: no nulls.
    pub valid: Option<&'a [u8]>,
}

impl<'a> From<Values<'a>> for Column<'a> {
    fn from(values: Values<'a>) -> Self {
        Self {
            values,
            valid: None,
        }
    }
}

/// A number a column is compared with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar {
    /// An integer.
    Int(i128),
    /// A float.
    Float(f64),
}

macro_rules! scalar_from {
    ($($t:ty => $v:ident),*) => {$(
        impl From<$t> for Scalar {
            fn from(x: $t) -> Self {
                Self::$v(x.into())
            }
        }
    )*};
}
scalar_from!(i64 => Int, i32 => Int, u64 => Int, u32 => Int, f64 => Float);

/// What a row must be to pass.
#[derive(Clone, Copy, Debug)]
pub enum Test<'a> {
    /// Less than.
    Lt(Scalar),
    /// Less than or equal.
    Le(Scalar),
    /// Greater than.
    Gt(Scalar),
    /// Greater than or equal.
    Ge(Scalar),
    /// Equal.
    Eq(Scalar),
    /// Between the two, both included: a time window.
    Between(Scalar, Scalar),
    /// One of these labels, given increasing; on a `u32` column only.
    In(&'a [u32]),
}

/// A test on each row of a column.
#[derive(Clone, Copy, Debug)]
pub struct Predicate<'a> {
    /// The column.
    pub column: Column<'a>,
    /// The test.
    pub test: Test<'a>,
}

impl<'a> Predicate<'a> {
    /// `test` on the rows of `column`.
    #[must_use]
    pub fn new(column: impl Into<Column<'a>>, test: Test<'a>) -> Self {
        Self {
            column: column.into(),
            test,
        }
    }

    /// Clears the bits of `words`, one per row of `rows`, whose row fails.
    fn and_into(&self, rows: u64, words: &mut [u64]) -> Result<(), Error> {
        if self.column.values.len() as u64 != rows {
            return Err(Error::Argument(
                "a column of a view has another number of rows than what it describes",
            ));
        }
        if matches!(self.test, Test::In(_)) && !matches!(self.column.values, Values::U32(_)) {
            return Err(Error::Argument("labels are tested on a u32 column"));
        }
        if let Test::In(set) = self.test {
            if !set.windows(2).all(|w| w[0] < w[1]) {
                return Err(Error::Argument("the labels of a test must be increasing"));
            }
        }
        match self.column.values {
            Values::F64(v) => and_test(v, self.test, words),
            Values::F32(v) => and_test(v, self.test, words),
            Values::I64(v) => and_test(v, self.test, words),
            Values::I32(v) => and_test(v, self.test, words),
            Values::U64(v) => and_test(v, self.test, words),
            Values::U32(v) => and_test(v, self.test, words),
        }
        if let Some(valid) = self.column.valid {
            if (valid.len() as u64) < rows.div_ceil(8) {
                return Err(Error::Argument("a validity bitmap shorter than its column"));
            }
            par::for_each_chunk_mut(words, GRAIN, |first, chunk| {
                for (w, word) in (first..).zip(chunk) {
                    let bytes = valid.get(8 * w..).unwrap_or_default();
                    let mut b = [0; 8];
                    let k = bytes.len().min(8);
                    b[..k].copy_from_slice(&bytes[..k]);
                    *word &= u64::from_le_bytes(b);
                }
            });
        }
        Ok(())
    }
}

/// A value of a column, compared exactly with a [`Scalar`].
trait Value: Copy + Sync {
    /// The order of `self` and `s`; `None` when one is a NaN.
    fn compare(self, s: Scalar) -> Option<Ordering>;
    /// The value as a label: for `u32` only.
    fn label(self) -> Option<u32> {
        None
    }
}

macro_rules! integer_value {
    ($($t:ty),*) => {$(
        impl Value for $t {
            #[inline]
            fn compare(self, s: Scalar) -> Option<Ordering> {
                let v = i128::from(self);
                match s {
                    Scalar::Int(i) => Some(v.cmp(&i)),
                    Scalar::Float(f) => int_float(v, f),
                }
            }
        }
    )*};
}
integer_value!(i64, i32, u64);

impl Value for u32 {
    #[inline]
    fn compare(self, s: Scalar) -> Option<Ordering> {
        u64::from(self).compare(s)
    }

    fn label(self) -> Option<u32> {
        Some(self)
    }
}

impl Value for f64 {
    #[inline]
    fn compare(self, s: Scalar) -> Option<Ordering> {
        match s {
            Scalar::Float(f) => self.partial_cmp(&f),
            Scalar::Int(i) => int_float(i, self).map(Ordering::reverse),
        }
    }
}

impl Value for f32 {
    #[inline]
    fn compare(self, s: Scalar) -> Option<Ordering> {
        f64::from(self).compare(s)
    }
}

/// The order of the integer `v` and the float `f`, exactly; `None` when `f` is a NaN.
fn int_float(v: i128, f: f64) -> Option<Ordering> {
    /// 2^127: every `i128` is below it, and at least −2^127.
    const LIMIT: f64 = 170_141_183_460_469_231_731_687_303_715_884_105_728.0;
    if f.is_nan() {
        return None;
    }
    if f >= LIMIT {
        return Some(Less);
    }
    if f < -LIMIT {
        return Some(Greater);
    }
    let whole = f.trunc();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a whole number within the range of i128"
    )]
    let t = whole as i128;
    // `f - whole` is exact: the fraction of a float is a float.
    Some(
        v.cmp(&t)
            .then(0.0_f64.partial_cmp(&(f - whole)).unwrap_or(Equal)),
    )
}

fn and_test<T: Value>(values: &[T], test: Test<'_>, words: &mut [u64]) {
    let is = |o: Option<Ordering>, a: Ordering, b: Ordering| o == Some(a) || o == Some(b);
    match test {
        Test::Lt(s) => and_rows(values, words, |v| v.compare(s) == Some(Less)),
        Test::Le(s) => and_rows(values, words, |v| is(v.compare(s), Less, Equal)),
        Test::Gt(s) => and_rows(values, words, |v| v.compare(s) == Some(Greater)),
        Test::Ge(s) => and_rows(values, words, |v| is(v.compare(s), Greater, Equal)),
        Test::Eq(s) => and_rows(values, words, |v| v.compare(s) == Some(Equal)),
        Test::Between(a, b) => and_rows(values, words, |v| {
            is(v.compare(a), Greater, Equal) && is(v.compare(b), Less, Equal)
        }),
        Test::In(set) => and_rows(values, words, |v| {
            v.label().is_some_and(|l| set.binary_search(&l).is_ok())
        }),
    }
}

/// Clears bit `i` of `words` where `keep(values[i])` is false, and the bits past the
/// values, in parallel. The test of a row sets its bit without a branch.
fn and_rows<T: Copy + Sync>(values: &[T], words: &mut [u64], keep: impl Fn(T) -> bool + Sync) {
    par::for_each_chunk_mut(words, GRAIN, |first, chunk| {
        for (w, word) in (first..).zip(chunk) {
            let rows = values.get(64 * w..).unwrap_or_default();
            let mut x = 0;
            for (b, &v) in rows.iter().take(64).enumerate() {
                x |= u64::from(keep(v)) << b;
            }
            *word &= x;
        }
    });
}

/// A `u64` as an index into memory.
fn ix(x: u64) -> Result<usize, Error> {
    usize::try_from(x).map_err(|_| Error::Invalid("a view larger than the address space"))
}

/// A position in a slice. One beyond the address space is beyond every slice, so the
/// index fails as for any position past the end.
fn row(x: u64) -> usize {
    usize::try_from(x).unwrap_or(usize::MAX)
}

/// Whether bit `i` of `bits` is set.
fn has(bits: &[u64], i: u64) -> bool {
    bits[row(i / 64)] >> (i % 64) & 1 == 1
}

/// The bits of the rows of `rows` that pass every predicate.
fn select(
    memory: &Memory,
    purpose: &'static str,
    rows: u64,
    predicates: &[Predicate<'_>],
) -> Result<SegVec<u64>, Error> {
    let mut bits = SegVec::try_with_len(memory, purpose, ix(rows.div_ceil(64))?, u64::MAX)?;
    let words = bits.as_mut_slice().ok_or(SEGMENT)?;
    for p in predicates {
        p.and_into(rows, words)?;
    }
    Ok(bits)
}

/// Checks the event offsets of an adjacency of `entries` entries: `entries + 1` of
/// them, from 0, never decreasing. Returns the number of events.
fn check_events(offsets: &[u64], entries: u64) -> Result<u64, Error> {
    let bad = Error::Argument("event offsets that are not those of the parent's entries");
    if offsets.len() as u64 != entries + 1 || offsets.first() != Some(&0) {
        return Err(bad);
    }
    let ok = par::map_ranges(offsets.len() - 1, GRAIN, |r| {
        offsets[r.start..=r.end].windows(2).all(|w| w[0] <= w[1])
    });
    if ok.into_iter().all(|x| x) {
        Ok(offsets[offsets.len() - 1])
    } else {
        Err(bad)
    }
}

/// Whether a bit of `range` is set in `bits`.
fn any(bits: &[u64], range: Range<u64>) -> bool {
    if range.is_empty() {
        return false;
    }
    let (a, b) = (range.start, range.end - 1);
    let (wa, wb) = (a / 64, b / 64);
    let (lo, hi) = (u64::MAX << (a % 64), u64::MAX >> (63 - b % 64));
    let word = |w: u64| usize::try_from(w).map_or(0, |w| bits[w]);
    if wa == wb {
        word(wa) & lo & hi != 0
    } else {
        word(wa) & lo != 0 || (wa + 1..wb).any(|w| word(w) != 0) || word(wb) & hi != 0
    }
}

/// A view's definition (module documentation). Making one reads nothing.
#[derive(Clone, Debug, Default)]
pub struct ViewSpec<'a> {
    nodes: Vec<Predicate<'a>>,
    sets: Vec<&'a [u32]>,
    pairs: Vec<Predicate<'a>>,
    events: Vec<(&'a [u64], Predicate<'a>)>,
}

impl<'a> ViewSpec<'a> {
    /// The view that keeps everything.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps only the nodes whose row of a node column passes `p`.
    #[must_use]
    pub fn nodes(mut self, p: Predicate<'a>) -> Self {
        self.nodes.push(p);
        self
    }

    /// Keeps only the nodes of `ids`, in any order.
    #[must_use]
    pub fn node_set(mut self, ids: &'a [u32]) -> Self {
        self.sets.push(ids);
        self
    }

    /// Keeps only the pairs whose row of a per-pair column passes `p`.
    #[must_use]
    pub fn pairs(mut self, p: Predicate<'a>) -> Self {
        self.pairs.push(p);
        self
    }

    /// Keeps only the events whose row of a per-event column passes `p`, and the pairs
    /// with a kept event. `offsets` are the event offsets of the parent's adjacency
    /// (`docs/spec/eg-adjacency.md` § 3.1): the events of entry `e` are the rows
    /// `offsets[e]..offsets[e + 1]`. Every event predicate reads the same offsets.
    #[must_use]
    pub fn events(mut self, offsets: &'a [u64], p: Predicate<'a>) -> Self {
        self.events.push((offsets, p));
        self
    }

    /// The view that `self` and `other` both keep.
    #[must_use]
    pub fn and(mut self, other: Self) -> Self {
        self.nodes.extend(other.nodes);
        self.sets.extend(other.sets);
        self.pairs.extend(other.pairs);
        self.events.extend(other.events);
        self
    }

    /// The view of `parent`: the predicates evaluated in `memory`, in parallel, then
    /// one pass over the parent's lists that counts the view's nodes and edges.
    ///
    /// # Errors
    ///
    /// [`Error::Argument`] for a column that does not have one row per node, pair or
    /// event, labels that do not increase, a node set that names a node outside the
    /// graph, or event offsets that are not those of the parent's entries;
    /// [`Error::Memory`] when the bitsets or the view do not fit; the parent's errors.
    pub fn build<'s, S: Source>(
        &self,
        parent: &'s S,
        memory: &Memory,
        settings: Settings,
    ) -> Result<Filtered<'s, 'a, S>, Error> {
        let (n, m) = (parent.nodes(), parent.entries());
        let nodes = if self.nodes.is_empty() && self.sets.is_empty() {
            None
        } else {
            let mut bits = select(memory, "view node bitset", n.into(), &self.nodes)?;
            let words = bits.as_mut_slice().ok_or(SEGMENT)?;
            for set in &self.sets {
                and_set(memory, n, set, words)?;
            }
            Some(bits)
        };
        let events = match self.events.first() {
            None => None,
            Some(&(offsets, _)) => {
                let same = |o: &[u64]| o.as_ptr() == offsets.as_ptr() && o.len() == offsets.len();
                if !self.events.iter().all(|(o, _)| same(o)) {
                    return Err(Error::Argument(
                        "the event predicates of a view read other event offsets",
                    ));
                }
                let rows = check_events(offsets, m)?;
                let tests: Vec<Predicate<'a>> = self.events.iter().map(|&(_, p)| p).collect();
                Some(Events {
                    offsets,
                    kept: select(memory, "view event bitset", rows, &tests)?,
                })
            }
        };
        let entries = if self.pairs.is_empty() && events.is_none() {
            None
        } else {
            let mut bits = select(memory, "view entry bitset", m, &self.pairs)?;
            if let Some(ev) = &events {
                let words = bits.as_mut_slice().ok_or(SEGMENT)?;
                let kept = ev.kept.as_slice().ok_or(SEGMENT)?;
                par::for_each_chunk_mut(words, GRAIN, |first, chunk| {
                    for (w, word) in (first as u64..).zip(chunk) {
                        let mut x = 0;
                        for e in (64 * w..m).take(64) {
                            let r = ev.offsets[row(e)]..ev.offsets[row(e) + 1];
                            x |= u64::from(any(kept, r)) << (e % 64);
                        }
                        *word &= x;
                    }
                });
            }
            Some(bits)
        };
        let view = View::new(parent, memory, settings, nodes, entries)?;
        Ok(Filtered { view, events })
    }
}

/// Clears the bits of `words` (one per node of `n`) of the nodes outside `ids`.
fn and_set(memory: &Memory, n: u32, ids: &[u32], words: &mut [u64]) -> Result<(), Error> {
    let mut set = SegVec::try_with_len(memory, "view node set", words.len(), 0_u64)?;
    let s = set.as_mut_slice().ok_or(SEGMENT)?;
    for &v in ids {
        if v >= n {
            return Err(Error::Argument("a node of a node set is not in the graph"));
        }
        s[ix((v / 64).into())?] |= 1 << (v % 64);
    }
    for (w, x) in words.iter_mut().zip(s) {
        *w &= *x;
    }
    Ok(())
}

/// The events a view keeps.
#[derive(Debug)]
struct Events<'a> {
    offsets: &'a [u64],
    /// Bit `i`: event `i` passes every event predicate.
    kept: SegVec<u64>,
}

/// A view built from a [`ViewSpec`]: the [`View`] the kernels run on, and the events it
/// keeps.
#[derive(Debug)]
pub struct Filtered<'s, 'a, S: Source> {
    view: View<'s, S>,
    events: Option<Events<'a>>,
}

impl<'s, S: Source> Filtered<'s, '_, S> {
    /// The view: a [`Source`] every kernel runs on.
    #[must_use]
    pub fn view(&self) -> &View<'s, S> {
        &self.view
    }

    /// Holds the view as the planner's rule says for work that reads every list
    /// `passes` times (`planner::passes` for an operation), with what `memory` has free:
    /// materialized or as a mask. Returns the form.
    ///
    /// # Errors
    ///
    /// As [`View::materialize`].
    pub fn hold(&mut self, passes: u32, memory: &Memory) -> Result<ViewForm, Error> {
        let form = planner::view_form(
            passes,
            self.view.nodes(),
            self.view.entries(),
            memory.available(),
        );
        match form {
            ViewForm::Materialized => self.view.materialize(memory)?,
            ViewForm::Mask => self.view.release(),
        }
        Ok(form)
    }

    /// The values of a per-pair column of the parent (one per entry), in the view's
    /// entry order: the weights a weighted kernel reads on the view.
    ///
    /// # Errors
    ///
    /// [`Error::Argument`] for a column of another length than the parent's entries;
    /// as [`View::gather`].
    pub fn pair_values(&self, column: &[f64], memory: &Memory) -> Result<SegVec<f64>, Error> {
        if column.len() as u64 != self.view.parent().entries() {
            return Err(Error::Argument(
                "a per-pair column of another length than the entries",
            ));
        }
        self.view
            .gather(memory, "view pair values", |e| column[row(e)])
    }

    /// The number of events of each pair of the view that the view keeps: all of them
    /// without event predicates. `offsets` are the event offsets of the parent's
    /// adjacency.
    ///
    /// # Errors
    ///
    /// As [`event_sums`](Self::event_sums).
    pub fn multiplicity(&self, offsets: &[u64], memory: &Memory) -> Result<SegVec<u64>, Error> {
        let kept = self.kept(offsets)?;
        self.view.gather(memory, "view multiplicity", |e| {
            let r = offsets[row(e)]..offsets[row(e) + 1];
            r.filter(|&i| kept.is_none_or(|k| has(k, i))).count() as u64
        })
    }

    /// The exact sum, rounded once (`graph-semantics.md` § 4.3), of a per-event column
    /// over the events of each pair of the view that the view keeps.
    ///
    /// # Errors
    ///
    /// [`Error::Argument`] for offsets that are not those of the parent's entries or
    /// differ in their events from the view's, or a column of another length than the
    /// events; as [`View::gather`].
    pub fn event_sums(
        &self,
        offsets: &[u64],
        column: &[f64],
        memory: &Memory,
    ) -> Result<SegVec<f64>, Error> {
        let kept = self.kept(offsets)?;
        if column.len() as u64 != offsets[offsets.len() - 1] {
            return Err(Error::Argument(
                "a per-event column of another length than the events",
            ));
        }
        self.view.gather(memory, "view event sums", |e| {
            let mut sum = ExactSum::new();
            for i in offsets[row(e)]..offsets[row(e) + 1] {
                if kept.is_none_or(|k| has(k, i)) {
                    sum.add(column[row(i)]);
                }
            }
            sum.value()
        })
    }

    /// Checks `offsets` against the parent and the view's events; the kept events'
    /// bits, if the view filters events.
    fn kept(&self, offsets: &[u64]) -> Result<Option<&[u64]>, Error> {
        let events = check_events(offsets, self.view.parent().entries())?;
        match &self.events {
            None => Ok(None),
            Some(ev) if ev.offsets.last() == Some(&events) => {
                ev.kept.as_slice().map(Some).ok_or(SEGMENT)
            }
            Some(_) => Err(Error::Argument(
                "event offsets other than those the view was built with",
            )),
        }
    }
}
