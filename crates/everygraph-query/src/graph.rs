// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The lists a query reads, their per-entry columns, and the per-thread [`Reader`].

use std::fmt;
use std::ops::ControlFlow;

use everygraph_core::{InsufficientBudget, Memory, SegVec, usize_of, widen};
use everygraph_format::adjacency::AdjacencyEvents;
use everygraph_kernels::edge_map::{Error as ListError, Source};

use crate::seen::Seen;
use crate::{Direction, QueryError};

/// The per-entry columns of one adjacency (`docs/spec/eg-columns.md` § 6), as slices
/// indexed by entry or event offset. Every one is optional; what a row reports follows
/// from what is given:
///
/// - `edges` of a pair: the per-pair `edges` column (`duplicates="sum"`), else the
///   length of its event range in `events` (`duplicates="events"`), else 1;
/// - the weight: the per-pair sum `weight`;
/// - `first` and `last`: the per-pair columns of that name, else the smallest and the
///   largest per-event `time` over the pair's events (one event per entry without
///   `events`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Columns<'a> {
    /// The `.events` section: where each entry's events are.
    pub events: Option<&'a AdjacencyEvents<'a>>,
    /// The multiplicity of each pair.
    pub edges: Option<&'a [u64]>,
    /// The weight of each pair.
    pub weight: Option<&'a [f64]>,
    /// The first time of each pair.
    pub first: Option<&'a [i64]>,
    /// The last time of each pair.
    pub last: Option<&'a [i64]>,
    /// The time of each event.
    pub time: Option<&'a [i64]>,
}

/// What a row reports about the events of one pair.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Value {
    pub edges: u64,
    pub weight: f64,
    pub first: i64,
    pub last: i64,
}

impl Value {
    /// Both directions of a pair of a directed graph, out first.
    fn and(self, other: Self) -> Self {
        Self {
            edges: self.edges + other.edges,
            weight: self.weight + other.weight,
            first: self.first.min(other.first),
            last: self.last.max(other.last),
        }
    }
}

/// Which columns rows carry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Shape {
    pub weight: bool,
    pub time: bool,
}

impl Shape {
    /// Bytes of a row of `edges_between` or `neighbors`: two nodes, the multiplicity,
    /// and the columns.
    pub fn row_bytes(self) -> u64 {
        16 + 8 * u64::from(self.weight) + 16 * u64::from(self.time)
    }
}

const NOT_ENTRIES: QueryError =
    QueryError::Invalid("a per-entry column whose length is not the number of entries");

fn index(e: u64) -> usize {
    usize::try_from(e).expect("an entry of a column held in memory")
}

impl Columns<'_> {
    fn check(&self, entries: u64) -> Result<(), QueryError> {
        let pairs = [
            self.edges.map(<[u64]>::len),
            self.weight.map(<[f64]>::len),
            self.first.map(<[i64]>::len),
            self.last.map(<[i64]>::len),
        ];
        if pairs.into_iter().flatten().any(|l| widen(l) != entries) {
            return Err(NOT_ENTRIES);
        }
        if self.first.is_some() != self.last.is_some() {
            return Err(QueryError::Invalid("`first` and `last` go together"));
        }
        let events = match self.events {
            Some(e) if e.entries() != entries => return Err(NOT_ENTRIES),
            Some(e) => {
                e.check().map_err(ListError::Format)?;
                e.events()
            }
            None => entries,
        };
        if self.time.is_some_and(|t| widen(t.len()) != events) {
            return Err(QueryError::Invalid(
                "a per-event column whose length is not the number of events",
            ));
        }
        Ok(())
    }

    fn shape(&self) -> Shape {
        Shape {
            weight: self.weight.is_some(),
            time: self.first.is_some() || self.time.is_some(),
        }
    }

    /// The value of entry `e`, with the columns of `want`.
    fn value(&self, e: u64, want: Shape) -> Result<Value, QueryError> {
        let events = match self.events {
            Some(ev) => Some(ev.event_range(e).map_err(ListError::Format)?),
            None => None,
        };
        let edges = match (self.edges, &events) {
            (Some(c), _) => c[index(e)],
            (None, Some(r)) => r.end - r.start,
            (None, None) => 1,
        };
        let weight = match self.weight {
            Some(w) if want.weight => w[index(e)],
            _ => 0.0,
        };
        let (first, last) = match (self.first, self.last, self.time) {
            _ if !want.time => (0, 0),
            (Some(f), Some(l), _) => (f[index(e)], l[index(e)]),
            (_, _, Some(t)) => {
                let r = events.unwrap_or(e..e + 1);
                let times = &t[index(r.start)..index(r.end)];
                let first = times.iter().copied().min().unwrap_or(i64::MAX);
                (first, times.iter().copied().max().unwrap_or(i64::MIN))
            }
            _ => (0, 0),
        };
        Ok(Value {
            edges,
            weight,
            first,
            last,
        })
    }
}

/// An empty column of the rows of a query, with room for `rows`.
pub(crate) fn column<T: Copy>(memory: &Memory, rows: usize) -> Result<SegVec<T>, QueryError> {
    let mut c = SegVec::new(memory, "point query rows");
    c.try_reserve(rows)?;
    Ok(c)
}

/// One adjacency.
struct Side<'a, S> {
    lists: &'a S,
    columns: Columns<'a>,
    /// The longest list.
    longest: u32,
}

impl<'a, S: Source> Side<'a, S> {
    fn new(lists: &'a S) -> Self {
        let mut longest = 0;
        lists.degrees(0..lists.nodes(), |_, d| longest = longest.max(d));
        Self {
            lists,
            columns: Columns::default(),
            longest,
        }
    }
}

/// The lists of a graph for point queries: the out-adjacency (the lists of an
/// undirected graph) and, on a directed graph, the in-adjacency when the file holds
/// one, which `In`, `Both` and the undirected projection read.
pub struct Graph<'a, S: Source> {
    out: Side<'a, S>,
    inn: Option<Side<'a, S>>,
    directed: bool,
    shape: Shape,
}

impl<S: Source> fmt::Debug for Graph<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Graph")
            .field("nodes", &self.nodes())
            .field("directed", &self.directed)
            .field("in_lists", &self.inn.is_some())
            .finish_non_exhaustive()
    }
}

impl<'a, S: Source> Graph<'a, S> {
    /// An undirected graph whose lists are `lists`. Reads every degree once.
    #[must_use]
    pub fn undirected(lists: &'a S) -> Self {
        Self {
            out: Side::new(lists),
            inn: None,
            directed: false,
            shape: Shape::default(),
        }
    }

    /// A directed graph: its out-lists, and its in-lists if the file holds them. Reads
    /// every degree once.
    ///
    /// # Errors
    ///
    /// [`QueryError::Invalid`] if the in-lists have other nodes or entries.
    pub fn directed(out: &'a S, inn: Option<&'a S>) -> Result<Self, QueryError> {
        if inn.is_some_and(|i| (i.nodes(), i.entries()) != (out.nodes(), out.entries())) {
            return Err(QueryError::Invalid(
                "the in-lists do not have the nodes and entries of the out-lists",
            ));
        }
        Ok(Self {
            out: Side::new(out),
            inn: inn.map(Side::new),
            directed: true,
            shape: Shape::default(),
        })
    }

    /// The graph with the per-entry columns of its out-lists, and of its in-lists, which
    /// must have the same columns.
    ///
    /// # Errors
    ///
    /// [`QueryError::Invalid`] for a column whose length does not fit, `first` without
    /// `last`, or in-lists whose columns are not those of the out-lists.
    /// [`QueryError::Lists`] for an `events` section that fails its check.
    pub fn with_columns(mut self, out: Columns<'a>, inn: Columns<'a>) -> Result<Self, QueryError> {
        out.check(self.out.lists.entries())?;
        if let Some(side) = &mut self.inn {
            inn.check(side.lists.entries())?;
            if inn.shape() != out.shape() || inn.edges.is_some() != out.edges.is_some() {
                return Err(QueryError::Invalid(
                    "the in-lists do not have the columns of the out-lists",
                ));
            }
            side.columns = inn;
        }
        self.out.columns = out;
        self.shape = out.shape();
        Ok(self)
    }

    /// Nodes.
    #[must_use]
    pub fn nodes(&self) -> u32 {
        self.out.lists.nodes()
    }

    /// Whether the graph is directed.
    #[must_use]
    pub fn is_directed(&self) -> bool {
        self.directed
    }

    /// Whether rows carry a weight.
    #[must_use]
    pub fn has_weight(&self) -> bool {
        self.shape.weight
    }

    /// Whether rows carry `first` and `last`.
    #[must_use]
    pub fn has_time(&self) -> bool {
        self.shape.time
    }

    /// A reader for one thread, whose buffers are held under `memory`, as the rows of
    /// its queries are.
    ///
    /// # Errors
    ///
    /// [`QueryError::Lists`] if its first buffers do not fit.
    pub fn reader(&self, memory: &Memory) -> Result<Reader<'_, 'a, S>, QueryError> {
        let make = |side: &Side<'a, S>| side.lists.reader(memory, BATCH, 1);
        Ok(Reader {
            graph: self,
            memory: memory.clone(),
            out: make(&self.out)?,
            inn: self.inn.as_ref().map(make).transpose()?,
            seen: Some(Seen::new(memory)),
        })
    }

    pub(crate) fn shape(&self) -> Shape {
        self.shape
    }

    /// `direction` as this graph reads it: `Out` on an undirected graph.
    ///
    /// # Errors
    ///
    /// [`QueryError::Unsupported`] for `In` and `Both` on a directed graph without
    /// in-lists.
    pub(crate) fn lists(&self, direction: Direction) -> Result<Direction, QueryError> {
        match direction {
            _ if !self.directed => Ok(Direction::Out),
            Direction::Out => Ok(Direction::Out),
            _ if self.inn.is_none() => Err(QueryError::Unsupported(NO_IN_LISTS)),
            d => Ok(d),
        }
    }

    /// The weight of entry `e` of the out-lists (`inn` false) or the in-lists; 0
    /// without a weight.
    pub(crate) fn weight(&self, inn: bool, e: u64) -> f64 {
        let side = if inn { self.inn.as_ref() } else { None };
        let columns = side.map_or(&self.out.columns, |s| &s.columns);
        columns.weight.map_or(0.0, |w| w[index(e)])
    }

    /// The value of entry `e` of the out-lists (`inn` false) or the in-lists.
    pub(crate) fn value(&self, inn: bool, e: u64, want: Shape) -> Result<Value, QueryError> {
        let side = if inn { self.inn.as_ref() } else { None };
        side.map_or(&self.out.columns, |s| &s.columns)
            .value(e, want)
    }

    /// The checked node `v`.
    pub(crate) fn node(&self, v: u32) -> Result<u32, QueryError> {
        if v < self.nodes() {
            Ok(v)
        } else {
            Err(QueryError::UnknownNode(v))
        }
    }

    /// The degree of `v` in `direction` (as [`lists`](Self::lists) gives it); for
    /// `Both`, out plus in, at least the distinct neighbours.
    pub(crate) fn degree(&self, v: u32, direction: Direction) -> u32 {
        let inn = || self.inn.as_ref().map_or(0, |s| s.lists.degree(v));
        match direction {
            Direction::Out => self.out.lists.degree(v),
            Direction::In => inn(),
            Direction::Both => self.out.lists.degree(v).saturating_add(inn()),
        }
    }

    /// The longest list in `direction`, per side.
    pub(crate) fn longest(&self, direction: Direction) -> u32 {
        let inn = self.inn.as_ref().map_or(0, |s| s.longest);
        match direction {
            Direction::Out => self.out.longest,
            Direction::In => inn,
            Direction::Both => self.out.longest.max(inn),
        }
    }
}

/// Why `In` and `Both` cannot be read.
pub(crate) const NO_IN_LISTS: &str =
    "this directed graph has no in-adjacency: its in-neighbours are not stored";

/// Entries a reader decodes at once, when it reads consecutive lists.
const BATCH: usize = 1 << 13;

/// What one thread needs to query a [`Graph`]: its decode buffers. Every query is a
/// method; the rows it returns are held under the reader's memory.
pub struct Reader<'g, 'a, S: Source> {
    pub(crate) graph: &'g Graph<'a, S>,
    pub(crate) memory: Memory,
    out: S::Reader,
    inn: Option<S::Reader>,
    /// The nodes a query met, kept for the next; taken while a query runs.
    seen: Option<Seen>,
}

impl<S: Source> fmt::Debug for Reader<'_, '_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reader")
            .field("graph", self.graph)
            .finish_non_exhaustive()
    }
}

impl<S: Source> Reader<'_, '_, S> {
    /// Runs `f` with the reader's set of nodes met, emptied for at most `most` nodes.
    pub(crate) fn with_seen<T>(
        &mut self,
        most: usize,
        f: impl FnOnce(&mut Self, &mut Seen) -> Result<T, QueryError>,
    ) -> Result<T, QueryError> {
        let mut seen = self.seen.take().unwrap_or_else(|| Seen::new(&self.memory));
        let out = seen
            .begin(self.graph.nodes(), most)
            .and_then(|()| f(self, &mut seen));
        self.seen = Some(seen);
        out
    }

    /// Refuses a query that may hold `state` bytes besides what its readers take to read
    /// parts of up to `part` entries of lists of up to `entries` entries in `direction`,
    /// before it reads anything. The readers' part is each source's own bound
    /// (`Source::list_bytes`).
    pub(crate) fn admit(
        &self,
        state: u64,
        direction: Direction,
        entries: u64,
        part: u64,
    ) -> Result<(), QueryError> {
        let g = self.graph;
        let part = part.min(entries);
        let side = |s: &Side<'_, S>| s.lists.list_bytes(entries, part, BATCH);
        let inn = g.inn.as_ref().map_or(0, side);
        let lists = match direction {
            Direction::Out => side(&g.out),
            Direction::In => inn,
            Direction::Both => side(&g.out).saturating_add(inn),
        };
        let need = state.saturating_add(lists);
        let available = self.memory.available();
        if need > available {
            return Err(QueryError::Budget(InsufficientBudget::new(
                "point query",
                need,
                available,
            )));
        }
        Ok(())
    }

    /// Calls `f(u, value)` for the first `k` distinct neighbours `u` of `v` in
    /// `direction` (as [`Graph::lists`] gives it), in graph order, with the columns of
    /// `want`; returns whether `v` has more. Under `Both` a pair's two directions are one
    /// row, and a kept loop counts once.
    pub(crate) fn gather(
        &mut self,
        v: u32,
        direction: Direction,
        k: usize,
        want: Shape,
        mut f: impl FnMut(u32, Value) -> Result<(), QueryError>,
    ) -> Result<bool, QueryError> {
        let g = self.graph;
        let out = &g.out;
        let Some((inn, reader)) = g.inn.as_ref().zip(self.inn.as_mut()) else {
            return one(out, &mut self.out, v, k, want, f);
        };
        match direction {
            Direction::Out => one(out, &mut self.out, v, k, want, f),
            Direction::In => one(inn, reader, v, k, want, f),
            Direction::Both => {
                let (fo, fi) = (out.lists.entry_offset(v), inn.lists.entry_offset(v));
                let (lo, more_out) = head(out.lists, &mut self.out, v, k)?;
                let (li, more_in) = head(inn.lists, reader, v, k)?;
                let mut union = Union::new(lo, li, v);
                for (u, i, j) in union.by_ref().take(k) {
                    let at = |side: &Side<'_, S>, first: u64, i: usize| {
                        side.columns.value(first + widen(i), want)
                    };
                    let value = match (i, j) {
                        (Some(i), Some(j)) => at(out, fo, i)?.and(at(inn, fi, j)?),
                        (Some(i), None) => at(out, fo, i)?,
                        (None, Some(j)) => at(inn, fi, j)?,
                        (None, None) => unreachable!("a node of the union is in a list"),
                    };
                    f(u, value)?;
                }
                Ok(more_out || more_in || !union.done())
            }
        }
    }

    /// Calls `f(u)` for each distinct neighbour `u` of `v` in `direction`, in graph
    /// order, until `f` breaks, and returns what it broke with. Each list is read in
    /// parts ([`Parts`]), only as far as `f` goes; the first part has `first` entries.
    /// Under `Both`, a node of both lists is given once.
    pub(crate) fn walk<B>(
        &mut self,
        v: u32,
        direction: Direction,
        first: usize,
        mut f: impl FnMut(u32) -> Result<ControlFlow<B>, QueryError>,
    ) -> Result<Option<B>, QueryError> {
        let g = self.graph;
        let Some((inn, reader)) = g.inn.as_ref().zip(self.inn.as_mut()) else {
            return walk_one(g.out.lists, &mut self.out, v, first, f);
        };
        match direction {
            Direction::Out => walk_one(g.out.lists, &mut self.out, v, first, f),
            Direction::In => walk_one(inn.lists, reader, v, first, f),
            Direction::Both => {
                let mut out_parts = Parts::new(g.out.lists, v, first);
                let mut in_parts = Parts::new(inn.lists, v, first);
                let (mut lo, mut li): (&[u32], &[u32]) = (&[], &[]);
                loop {
                    if lo.is_empty() {
                        lo = out_parts.next(g.out.lists, &mut self.out)?;
                    }
                    if li.is_empty() {
                        li = in_parts.next(inn.lists, reader)?;
                    }
                    let (a, b) = (lo.first().copied(), li.first().copied());
                    let Some(u) = a.into_iter().chain(b).min() else {
                        return Ok(None);
                    };
                    lo = &lo[usize::from(a == Some(u))..];
                    li = &li[usize::from(b == Some(u))..];
                    if let ControlFlow::Break(x) = f(u)? {
                        return Ok(Some(x));
                    }
                }
            }
        }
    }

    /// The first entry and the whole list of `v` in `direction`, `Out` or `In`.
    pub(crate) fn list(
        &mut self,
        v: u32,
        direction: Direction,
    ) -> Result<(u64, &[u32]), QueryError> {
        let g = self.graph;
        Ok(match (direction, g.inn.as_ref().zip(self.inn.as_mut())) {
            (Direction::In, Some((inn, reader))) => inn.lists.list(reader, v)?,
            (Direction::In, None) => return Err(QueryError::Unsupported(NO_IN_LISTS)),
            _ => g.out.lists.list(&mut self.out, v)?,
        })
    }

    /// The out- and in-lists of `v` with their first entries, read at once.
    pub(crate) fn both(&mut self, v: u32) -> Result<[(u64, &[u32]); 2], QueryError> {
        let g = self.graph;
        let (inn, reader) = g
            .inn
            .as_ref()
            .zip(self.inn.as_mut())
            .ok_or(QueryError::Unsupported(NO_IN_LISTS))?;
        let out = g.out.lists.list(&mut self.out, v)?;
        Ok([out, inn.lists.list(reader, v)?])
    }
}

/// [`Reader::gather`] on one adjacency.
fn one<S: Source>(
    side: &Side<'_, S>,
    reader: &mut S::Reader,
    v: u32,
    k: usize,
    want: Shape,
    mut f: impl FnMut(u32, Value) -> Result<(), QueryError>,
) -> Result<bool, QueryError> {
    let first = side.lists.entry_offset(v);
    let (list, more) = head(side.lists, reader, v, k)?;
    for (e, &u) in (first..).zip(list) {
        f(u, side.columns.value(e, want)?)?;
    }
    Ok(more)
}

/// The first `k` entries of the list of `v`, all of them if it has fewer, and whether
/// it has more.
fn head<'r, S: Source>(
    lists: &'r S,
    reader: &'r mut S::Reader,
    v: u32,
    k: usize,
) -> Result<(&'r [u32], bool), QueryError> {
    let degree = usize_of(lists.degree(v));
    let to = k.min(degree);
    let list = if to == 0 {
        &[]
    } else {
        lists.list_part(reader, v, (0, 0), to)?
    };
    Ok((list, degree > k))
}

/// [`Reader::walk`] on one adjacency.
fn walk_one<S: Source, B>(
    lists: &S,
    reader: &mut S::Reader,
    v: u32,
    first: usize,
    mut f: impl FnMut(u32) -> Result<ControlFlow<B>, QueryError>,
) -> Result<Option<B>, QueryError> {
    let mut parts = Parts::new(lists, v, first);
    loop {
        let part = parts.next(lists, reader)?;
        if part.is_empty() {
            return Ok(None);
        }
        for &u in part {
            if let ControlFlow::Break(x) = f(u)? {
                return Ok(Some(x));
            }
        }
    }
}

/// The list of one node, read in parts by `Source::list_part`: the first `first`
/// entries (rounded up to a multiple of four), then each part as long as all before
/// it. A decoding source decodes each entry once.
struct Parts {
    v: u32,
    first: usize,
    degree: usize,
    /// The first entry not read.
    at: usize,
    /// Entry `at − 1`.
    prev: u32,
}

impl Parts {
    fn new<S: Source>(lists: &S, v: u32, first: usize) -> Self {
        let degree = usize_of(lists.degree(v));
        let first = first.clamp(1, degree.max(1)).checked_next_multiple_of(4);
        Self {
            v,
            first: first.unwrap_or(usize::MAX),
            degree,
            at: 0,
            prev: 0,
        }
    }

    /// The next part; empty after the last.
    fn next<'r, S: Source>(
        &mut self,
        lists: &'r S,
        reader: &'r mut S::Reader,
    ) -> Result<&'r [u32], QueryError> {
        if self.at == self.degree {
            return Ok(&[]);
        }
        let to = match self.at {
            0 => self.first,
            at => at.saturating_mul(2),
        };
        let to = to.min(self.degree);
        let part = lists.list_part(reader, self.v, (self.at, self.prev), to)?;
        self.at = to;
        self.prev = part.last().copied().unwrap_or(self.prev);
        Ok(part)
    }
}

/// The union of the out-list and the in-list of `v`, both sorted: each node once, with
/// its position in each list. A kept loop is taken from the out-list only.
pub(crate) struct Union<'l> {
    out: &'l [u32],
    inn: &'l [u32],
    v: u32,
    i: usize,
    j: usize,
}

impl<'l> Union<'l> {
    pub(crate) fn new(out: &'l [u32], inn: &'l [u32], v: u32) -> Self {
        Self {
            out,
            inn,
            v,
            i: 0,
            j: 0,
        }
    }

    /// Whether every node of both lists has been given.
    pub(crate) fn done(&self) -> bool {
        self.i == self.out.len() && self.j == self.inn.len()
    }
}

impl Iterator for Union<'_> {
    type Item = (u32, Option<usize>, Option<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        let (a, b) = (self.out.get(self.i).copied(), self.inn.get(self.j).copied());
        let u = a.into_iter().chain(b).min()?;
        let i = (a == Some(u)).then_some(self.i);
        let j = (b == Some(u)).then_some(self.j);
        self.i += usize::from(i.is_some());
        self.j += usize::from(j.is_some());
        let j = j.filter(|_| u != self.v || i.is_none());
        Some((u, i, j))
    }
}
