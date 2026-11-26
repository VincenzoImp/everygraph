// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The whole-graph algorithms of `docs/api-sketch.md` § 3.2, each a call of its kernel
//! in `everygraph-kernels` with the parameters it was given.
//!
//! An undirected graph's kernels read its one adjacency. A directed graph's read its
//! out-adjacency, its in-adjacency (`PageRank` pulls along in-lists; BFS steps bottom-up
//! along them, or follows them with `direction="in"`), or the undirected projection
//! merged from both (core numbers, `degree`, BFS with `direction="both"`). A file built
//! without its in-adjacency answers only what the out-lists answer.

use std::sync::atomic::AtomicU32;

use arrow_array::ArrayRef;
use arrow_array::types::{Float32Type, Float64Type, UInt32Type};
use everygraph_core::MemoryError;
use everygraph_core::{Budget, ByteSource, Memory, MemoryKind, Reservation, SegVec};
use everygraph_format::adjacency::{
    Adjacency, AdjacencyEvents, AdjacencyIndex, EncodeError, merge, union_degrees,
};
use everygraph_format::container::Section;
use everygraph_ingest::graph::OpenError;
use everygraph_kernels::components::{self, WccOptions};
use everygraph_kernels::degrees::{self, Loop};
use everygraph_kernels::edge_map::{Compressed, Error, Plan, Settings, Source, one_segment_mut};
use everygraph_kernels::kcore;
use everygraph_kernels::pagerank::{self, PageRank};
use everygraph_kernels::traversal::{self, Bfs, UNREACHED};
use everygraph_kernels::{Atomics, scc};

use crate::error::{self, Failure};
use crate::graph::Shared;
use crate::table::{self, NULL};

/// A value the result's `meta` gains.
pub(crate) enum Extra {
    Count(u64),
    Real(f64),
    Flag(bool),
}

/// What a kernel returns: its columns after `node`, the identifier columns after those,
/// and what its `meta` gains.
pub(crate) struct Output {
    pub(crate) columns: Vec<(&'static str, Vec<ArrayRef>)>,
    pub(crate) ids: Vec<(&'static str, Vec<u32>)>,
    pub(crate) extra: Vec<(&'static str, Extra)>,
}

/// One call: its operation and the budget of its state.
pub(crate) struct Call<'a> {
    pub(crate) operation: &'static str,
    pub(crate) budget: &'a Budget,
    pub(crate) memory: &'a Memory,
}

impl Call<'_> {
    fn fail(&self, e: Error) -> Failure {
        error::kernel(self.operation, e, self.budget.limit())
    }

    /// Runs `f` on a plan over the out-adjacency of `g`.
    fn on_lists<R>(
        &self,
        g: &Shared,
        f: impl for<'g> FnOnce(&Plan<'_, 'g, Compressed<'_>>) -> Result<R, Error>,
    ) -> Result<R, Failure> {
        let data = g.file().data().map_err(|e| corrupt("adjacency.out", &e))?;
        let lists = Compressed::mapped(g.file().index(), &data).map_err(|e| self.fail(e))?;
        Plan::with(&lists, self.memory, Settings::default(), |plan| f(&plan))
            .map_err(|e| self.fail(e))
    }

    /// Runs `f` on plans over the out- and in-adjacency of the directed graph `g`.
    fn on_pair<R>(
        &self,
        g: &Shared,
        f: impl for<'g, 'c> FnOnce(
            &Plan<'_, 'g, Compressed<'c>>,
            &Plan<'_, 'g, Compressed<'c>>,
        ) -> Result<R, Error>,
    ) -> Result<R, Failure> {
        let file = g.file();
        let in_index = file.in_index().ok_or_else(|| self.needs_in_edges())?;
        let out_data = file.data().map_err(|e| corrupt("adjacency.out", &e))?;
        let in_data = file.in_data().map_err(|e| corrupt("adjacency.in", &e))?;
        let out = Compressed::mapped(file.index(), &out_data).map_err(|e| self.fail(e))?;
        let inn = Compressed::mapped(in_index, &in_data).map_err(|e| self.fail(e))?;
        // One type for both, so that a kernel can take either as either argument.
        let [out, inn] = [out, inn];
        Plan::with_pair(&out, &inn, self.memory, Settings::default(), |po, pi| {
            f(&po, &pi)
        })
        .and_then(|r| r)
        .map_err(|e| self.fail(e))
    }

    /// Runs `f` on the out- and in-adjacency of the directed graph `g`, both in memory.
    fn on_both<R>(
        &self,
        g: &Shared,
        f: impl FnOnce(&Adjacency<'_>, &Adjacency<'_>) -> Result<R, EncodeError>,
    ) -> Result<R, Failure> {
        let file = g.file();
        let in_index = file.in_index().ok_or_else(|| self.needs_in_edges())?;
        let out_data = file.data().map_err(|e| corrupt("adjacency.out", &e))?;
        let in_data = file.in_data().map_err(|e| corrupt("adjacency.in", &e))?;
        let out = in_memory(file.index(), &out_data, "adjacency.out")?;
        let inn = in_memory(in_index, &in_data, "adjacency.in")?;
        f(&out, &inn).map_err(|e| match e {
            EncodeError::OutOfMemory { bytes } => {
                self.fail(Error::Memory(MemoryError::AllocationFailed {
                    purpose: "undirected projection",
                    bytes,
                }))
            }
            other => corrupt("adjacency", &other),
        })
    }

    /// Runs `f` on a plan over the undirected projection of the directed graph `g`,
    /// merged in memory from its out- and in-adjacency.
    fn on_projection<R>(
        &self,
        g: &Shared,
        f: impl for<'g> FnOnce(&Plan<'_, 'g, Compressed<'_>>) -> Result<R, Error>,
    ) -> Result<R, Failure> {
        let (merged, mut held) = self.on_both(g, |out, inn| merge(out, inn, self.memory))?;
        let corrupt = |e: &dyn std::fmt::Display| corrupt("undirected projection", e);
        // The parsed index beside its bytes, then alone.
        let bytes = u64::try_from(merged.index.len()).unwrap_or(u64::MAX);
        held.try_grow(bytes.saturating_mul(2))
            .map_err(|e| self.fail(Error::Memory(e.into())))?;
        let index = AdjacencyIndex::parse(&merged.index).map_err(|e| corrupt(&e))?;
        let parsed = u64::try_from(index.heap_bytes()).unwrap_or(u64::MAX);
        held.shrink(bytes.saturating_mul(2).saturating_sub(parsed));
        let adjacency = Adjacency::new(&index, &merged.data).map_err(|e| corrupt(&e))?;
        let lists = Compressed::new(adjacency);
        Plan::with(&lists, self.memory, Settings::default(), |plan| f(&plan))
            .map_err(|e| self.fail(e))
    }

    /// The refusal of an operation that needs the in-adjacency of a directed graph,
    /// for a file built without it.
    fn needs_in_edges(&self) -> Failure {
        Failure::new("Unsupported")
            .text("operation", self.operation)
            .text(
                "reason",
                "it needs the in-edges of a directed graph, and this file was built without them",
            )
    }

    /// One weight per entry of an adjacency: the number of edges of its pair, when some
    /// pair repeats and `weight` is asked for (`docs/api-sketch.md` § 2.4); `None` when
    /// every pair counts 1. `events` opens the adjacency's events, with the reservation
    /// of their sequence, only when weights are asked for.
    fn pair_weights<'a>(
        &self,
        events: impl FnOnce() -> Result<Option<(AdjacencyEvents<'a>, Reservation)>, OpenError>,
        section: &str,
        weight: bool,
    ) -> Result<Option<SegVec<f64>>, Failure> {
        if !weight {
            return Ok(None);
        }
        let events = events().map_err(|e| match e {
            OpenError::Budget(refused) => self.fail(Error::Memory(refused.into())),
            e => corrupt(section, &e),
        })?;
        let Some((events, _held)) = events else {
            return Ok(None);
        };
        let entries =
            usize::try_from(events.entries()).map_err(|_| corrupt(section, &"too long"))?;
        let mut w = SegVec::try_with_len(self.memory, "pair weights", entries, 0.0)
            .map_err(|e| self.fail(Error::Memory(e)))?;
        let out = one_segment_mut(&mut w).map_err(|e| self.fail(e))?;
        let counts = events.multiplicities().map_err(|e| corrupt(section, &e))?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a count of events, exact below 2^53"
        )]
        for (slot, m) in out.iter_mut().zip(counts) {
            *slot = m as f64;
        }
        Ok(Some(w))
    }
}

/// The adjacency of `index` over `data`, a section held in memory.
fn in_memory<'a, S: ByteSource>(
    index: &'a AdjacencyIndex,
    data: &'a Section<'_, S>,
    name: &str,
) -> Result<Adjacency<'a>, Failure> {
    let bytes = data
        .as_slice()
        .ok_or_else(|| corrupt(name, &"not held in memory"))?;
    Adjacency::new(index, bytes).map_err(|e| corrupt(name, &e))
}

fn corrupt(section: &str, e: &dyn std::fmt::Display) -> Failure {
    Failure::new("CorruptFile")
        .text("section", section)
        .text("why", e)
}

/// `PageRank` (#47): `pagerank`, f64, or f32 with `low`.
pub(crate) fn pagerank(
    call: &Call<'_>,
    g: &Shared,
    p: &PageRank<'_>,
    weight: bool,
    low: bool,
) -> Result<Output, Failure> {
    let file = g.file();
    let out_weights =
        call.pair_weights(|| file.events(call.memory), "adjacency.out.events", weight)?;
    let out_weights = out_weights.as_ref().and_then(SegVec::as_slice);
    let run = Ranks {
        p,
        low,
        memory: call.memory,
    };
    if !file.directed() {
        return call.on_lists(g, |plan| run.on(plan, plan, [out_weights; 2], false));
    }
    let in_weights = call.pair_weights(
        || file.in_events(call.memory),
        "adjacency.in.events",
        weight,
    )?;
    let in_weights = in_weights.as_ref().and_then(SegVec::as_slice);
    call.on_pair(g, |out, inn| {
        run.on(out, inn, [out_weights, in_weights], true)
    })
}

/// The parameters of one `PageRank` call.
struct Ranks<'a> {
    p: &'a PageRank<'a>,
    low: bool,
    memory: &'a Memory,
}

impl Ranks<'_> {
    /// Out-strengths from `out`, then the iteration pulling along `inn`, with the
    /// per-entry weights of each.
    fn on<'g, S: Source>(
        &self,
        out: &Plan<'_, 'g, S>,
        inn: &Plan<'_, 'g, S>,
        [out_weights, in_weights]: [Option<&[f64]>; 2],
        directed: bool,
    ) -> Result<Output, Error> {
        let (p, memory) = (self.p, self.memory);
        let strength = pagerank::strength(out, out_weights, directed, memory)?;
        let s = strength.as_slice().ok_or(Error::Invalid("one segment"))?;
        let graph = pagerank::Graph {
            lists: inn,
            weights: in_weights,
            directed,
        };
        let (column, iterations, change) = if self.low {
            let r = pagerank::pagerank::<_, f32>(&graph, s, p, memory)?;
            (table::lend::<Float32Type>(r.values), r.iterations, r.change)
        } else {
            let r = pagerank::pagerank::<_, f64>(&graph, s, p, memory)?;
            (table::lend::<Float64Type>(r.values), r.iterations, r.change)
        };
        Ok(Output {
            columns: vec![("pagerank", column)],
            ids: Vec::new(),
            extra: vec![
                ("iterations", Extra::Count(u64::from(iterations))),
                ("converged", Extra::Flag(change < p.tol)),
                ("change", Extra::Real(change)),
            ],
        })
    }
}

/// BFS (#49) from the dense id `source`: `distance` (null where unreached) and
/// `parent`, the earliest node in graph order one hop closer to the source.
pub(crate) fn bfs(
    call: &Call<'_>,
    g: &Shared,
    source: u32,
    direction: &str,
    max_hops: Option<u32>,
) -> Result<Output, Failure> {
    let p = Bfs {
        max_hops,
        parents: true,
    };
    let memory = call.memory;
    let file = g.file();
    match direction {
        _ if !file.directed() => {
            call.on_lists(g, |plan| bfs_on(plan, Some(plan), source, &p, memory))
        }
        "in" => call.on_pair(g, |out, inn| bfs_on(inn, Some(out), source, &p, memory)),
        "both" => call.on_projection(g, |plan| bfs_on(plan, Some(plan), source, &p, memory)),
        // Top-down only along the out-lists of a file built without its in-adjacency.
        _ if file.in_index().is_none() => {
            call.on_lists(g, |plan| bfs_on(plan, None, source, &p, memory))
        }
        _ => call.on_pair(g, |out, inn| bfs_on(out, Some(inn), source, &p, memory)),
    }
}

/// BFS along `lists`, bottom-up steps looking along `back` (the reverse lists).
fn bfs_on<'g, S: Source>(
    lists: &Plan<'_, 'g, S>,
    back: Option<&Plan<'_, 'g, S>>,
    source: u32,
    p: &Bfs,
    memory: &Memory,
) -> Result<Output, Error> {
    let traversal::Hops {
        distance,
        parent,
        levels,
        held: state,
    } = traversal::bfs(lists, back, source, p, memory)?;
    let deepest = levels.len().saturating_sub(1) as u64;
    // The result takes over the state's arrays: same layout, collected in place.
    drop(state);
    let held = memory.reserve(
        "bfs result",
        MemoryKind::Anonymous,
        // Distances and parents, and the bitmap of the unreached.
        distance.len() as u64 * 8 + distance.len() as u64 / 8 + 1,
    )?;
    let distance: Vec<u32> = distance.into_iter().map(AtomicU32::into_inner).collect();
    let parent: Vec<u32> = parent
        .unwrap_or_default()
        .into_iter()
        .map(|p| match p.into_inner() {
            UNREACHED => NULL,
            p => p,
        })
        .collect();
    Ok(Output {
        columns: vec![("distance", table::own_u32(distance, Some(UNREACHED), held))],
        ids: vec![("parent", parent)],
        extra: vec![("hops", Extra::Count(deepest))],
    })
}

/// Weakly connected components (#48), or with `strong` the strongly connected ones of a
/// directed graph (#223): `component`, numbered from 0 by decreasing size, ties by the
/// earliest node (`docs/api-sketch.md` § 2.12).
pub(crate) fn components(call: &Call<'_>, g: &Shared, strong: bool) -> Result<Output, Failure> {
    let memory = call.memory;
    let numbered = |(labels, held): Atomics| {
        let numbered = components::number(&labels, memory)?;
        drop((labels, held));
        let count = numbered.sizes.len() as u64;
        Ok(Output {
            columns: vec![("component", table::lend::<UInt32Type>(numbered.component))],
            ids: Vec::new(),
            extra: vec![("components", Extra::Count(count))],
        })
    };
    if strong {
        return call.on_pair(g, |out, inn| numbered(scc::scc(out, inn, memory)?));
    }
    let options = WccOptions {
        symmetric: !g.file().directed(),
        sample: true,
    };
    call.on_lists(g, |plan| numbered(components::wcc(plan, memory, options)?))
}

/// Core numbers (#50): `core`, direction ignored: on a directed graph, those of its
/// undirected projection.
pub(crate) fn kcore(call: &Call<'_>, g: &Shared) -> Result<Output, Failure> {
    let memory = call.memory;
    let run = |plan: &Plan<'_, '_, Compressed<'_>>| {
        let (core, state) = kcore::kcore(plan, memory)?;
        drop(state);
        let held = memory.reserve("core numbers", MemoryKind::Anonymous, core.len() as u64 * 4)?;
        let core: Vec<u32> = core.into_iter().map(AtomicU32::into_inner).collect();
        let largest = core.iter().copied().max().unwrap_or(0);
        Ok(Output {
            columns: vec![("core", table::own_u32(core, None, held))],
            ids: Vec::new(),
            extra: vec![("largest_core", Extra::Count(u64::from(largest)))],
        })
    };
    if g.file().directed() {
        call.on_projection(g, run)
    } else {
        call.on_lists(g, run)
    }
}

/// Degrees (#50), the distinct neighbours of each node (`docs/api-sketch.md` § 3.2):
/// `degree` on an undirected graph; `in_degree`, `out_degree` and `degree` (in the
/// undirected projection) on a directed one.
pub(crate) fn degrees(call: &Call<'_>, g: &Shared) -> Result<Output, Failure> {
    let memory = call.memory;
    let output = |columns| Output {
        columns,
        ids: Vec::new(),
        extra: Vec::new(),
    };
    let count = |plan: &Plan<'_, '_, Compressed<'_>>, own| {
        degrees::degrees(plan, memory, own).map(table::lend::<UInt32Type>)
    };
    if !g.file().directed() {
        return call.on_lists(g, |plan| {
            Ok(output(vec![("degree", count(plan, Loop::Twice)?)]))
        });
    }
    let (in_degree, out_degree) = call.on_pair(g, |out, inn| {
        Ok((count(inn, Loop::Once)?, count(out, Loop::Once)?))
    })?;
    let n = usize::try_from(g.file().nodes()).unwrap_or(usize::MAX);
    let mut d = SegVec::try_with_len(memory, "degrees", n, 0_u32)
        .map_err(|e| call.fail(Error::Memory(e)))?;
    let slots = one_segment_mut(&mut d).map_err(|e| call.fail(e))?;
    call.on_both(g, |out, inn| union_degrees(out, inn, memory, slots))?;
    let degree = table::lend::<UInt32Type>(d);
    Ok(output(vec![
        ("in_degree", in_degree),
        ("out_degree", out_degree),
        ("degree", degree),
    ]))
}
