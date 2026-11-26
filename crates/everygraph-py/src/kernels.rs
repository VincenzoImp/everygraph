// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The whole-graph algorithms of `docs/api-sketch.md` § 3.2 on a graph's out-adjacency,
//! each a call of its kernel in `everygraph-kernels` with the parameters it was given.
//!
//! The in-adjacency is not stored yet (`m2-11-in-adjacency`), so on a directed graph
//! only what the out-lists answer is offered: components, and BFS along out-edges.

use std::sync::atomic::AtomicU32;

use arrow_array::ArrayRef;
use arrow_array::types::{Float32Type, Float64Type};
use everygraph_core::{Budget, Memory, MemoryKind, SegVec};
use everygraph_kernels::edge_map::{Compressed, Error, Plan, Settings, one_segment_mut};
use everygraph_kernels::pagerank::{self, PageRank};
use everygraph_kernels::traversal::{self, Bfs, UNREACHED};

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

    /// The refusal of an operation that needs the in-adjacency of a directed graph.
    fn needs_in_edges(&self) -> Failure {
        Failure::new("Unsupported")
            .text("operation", self.operation)
            .text(
                "reason",
                "it needs the in-edges of a directed graph, which are not stored yet",
            )
    }

    /// One weight per entry of the out-adjacency: the number of edges of its pair, when
    /// some pair repeats and `weight` is asked for (`docs/api-sketch.md` § 2.4); `None`
    /// when every pair counts 1.
    fn pair_weights(&self, g: &Shared, weight: bool) -> Result<Option<SegVec<f64>>, Failure> {
        const SECTION: &str = "adjacency.out.events";
        let events = g.file().events().map_err(|e| corrupt(SECTION, &e))?;
        let Some(events) = events.filter(|_| weight) else {
            return Ok(None);
        };
        let entries =
            usize::try_from(events.entries()).map_err(|_| corrupt(SECTION, &"too long"))?;
        let mut w = SegVec::try_with_len(self.memory, "pair weights", entries, 0.0)
            .map_err(|e| self.fail(Error::Memory(e)))?;
        let out = one_segment_mut(&mut w).map_err(|e| self.fail(e))?;
        let counts = events.multiplicities().map_err(|e| corrupt(SECTION, &e))?;
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
    if g.file().directed() {
        return Err(call.needs_in_edges());
    }
    let weights = call.pair_weights(g, weight)?;
    let weights = weights.as_ref().and_then(SegVec::as_slice);
    let memory = call.memory;
    call.on_lists(g, |plan| {
        let strength = pagerank::strength(plan, weights, false, memory)?;
        let s = strength.as_slice().ok_or(Error::Invalid("one segment"))?;
        let graph = pagerank::Graph {
            lists: plan,
            weights,
            directed: false,
        };
        let (column, iterations, change) = if low {
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
    })
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
    let directed = g.file().directed();
    if directed && direction != "out" {
        return Err(call.needs_in_edges());
    }
    let p = Bfs {
        max_hops,
        parents: true,
    };
    let memory = call.memory;
    call.on_lists(g, |plan| {
        let inn = (!directed).then_some(plan);
        let traversal::Hops {
            distance,
            parent,
            levels,
            held: state,
        } = traversal::bfs(plan, inn, source, &p, memory)?;
        let deepest = levels.len().saturating_sub(1) as u64;
        // The result takes over the state's arrays: same layout, collected in place.
        drop(state);
        let held = memory.reserve(
            "bfs result",
            MemoryKind::Anonymous,
            distance.len() as u64 * 8,
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
            columns: vec![("distance", table::own_u32(distance, UNREACHED, held))],
            ids: vec![("parent", parent)],
            extra: vec![("hops", Extra::Count(deepest))],
        })
    })
}
