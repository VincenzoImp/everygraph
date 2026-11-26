// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`check`]: the conservation checks of `everygraph verify --deep` on every stored
//! hierarchy and map tree (`docs/spec/eg-communities.md` § 3, § 6).

use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicU32, AtomicU64};

use everygraph_core::rng::{Purpose, draw};
use everygraph_core::{ByteSource, Memory, ReadError, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex};
use everygraph_format::container::{Container, ContainerError};
use everygraph_format::verify::Report;
use everygraph_kernels::edge_map::{Compressed, Error, Plan, Settings, Source, all};

use super::{Hierarchy, HierarchyError, OPERATION};
use crate::column::{Column, GRAIN};
use crate::level::Weights;
use crate::map;
use crate::quality::{Table, tally};

/// Checks every hierarchy stored for the graph of `file` (held in memory: a memory
/// map), and adds what is wrong to `report`:
///
/// - at every level, node counts that add up to the graph's nodes and to those of the
///   level above, and communities numbered by decreasing node count, then smallest node;
/// - the table of every level equal to that of its quotient graph under singletons, and
///   to that of the partition its parents make of the level below; the finest level's
///   to that of the membership of the graph (`adjacency.out`, when undirected). So the
///   volumes add up to `2W` and the internal and cut weights to `W` at every level;
/// - the entries of every quotient graph equal those that its level's partition makes
///   of the level below (or of the graph), through a random weight per community: for
///   each community `a`, `Σ_b A[a][b]·h(b)` modulo a prime near 2^64 over the stored
///   entries against the same sum over the level below, mapped. A changed entry passes
///   with probability about `2^-64`. The weights are drawn from a fixed seed, so the
///   check is the same on every run.
///
/// A map tree (`map` results) is checked against the rules of
/// `docs/spec/eg-communities.md` § 6: its tree and kinds, its counts against its
/// children and its sibling graph, its leaves and sibling graph against the graph.
///
/// What the budget does not hold, and results whose records cannot be read, are listed
/// as unchecked.
///
/// # Errors
///
/// [`ReadError::Backend`] when the file cannot be read.
pub fn check<S: ByteSource + Sync>(
    file: &Container<S>,
    memory: &Memory,
    report: &mut Report,
) -> Result<(), ReadError> {
    let listed = match file.results() {
        Ok(listed) => listed,
        Err(ContainerError::Read(e @ ReadError::Backend(_))) => return Err(e),
        Err(e) => {
            report
                .unchecked
                .push(format!("{OPERATION}: the stored results ({e})"));
            return Ok(());
        }
    };
    let stored = |operation: &str| operation == OPERATION || operation == map::OPERATION;
    for result in listed
        .iter()
        .filter(|r| stored(&r.provenance.operation) && r.of_this_graph)
    {
        let outcome = file
            .listed_result(result)
            .map_err(HierarchyError::File)
            .and_then(|root| {
                if result.provenance.operation == map::OPERATION {
                    if !map::check(file, root, &result.provenance, memory)? {
                        report.unchecked.push(format!(
                            "{}: against the graph (no undirected adjacency.out in memory)",
                            result.name
                        ));
                    }
                    return Ok(());
                }
                levels(file, &Hierarchy::of(file, root)?, memory, report)
            });
        match outcome {
            Ok(()) => {}
            Err(HierarchyError::File(ContainerError::Read(e @ ReadError::Backend(_))))
            | Err(HierarchyError::Graph(Error::Read(e @ ReadError::Backend(_)))) => return Err(e),
            Err(HierarchyError::Graph(Error::Memory(_))) => report
                .unchecked
                .push(format!("{}: not enough memory", result.name)),
            Err(e) => report.problem(Some(&result.name), e.to_string()),
        }
    }
    Ok(())
}

fn fail(level: u32, what: &str) -> HierarchyError {
    HierarchyError::Invalid(format!("level {level}: {what}"))
}

/// The total weight of a quotient past which its sums could overflow: Leiden's limit.
const MAX_EVENTS: u64 = 1 << 62;
/// The largest prime below 2^64. The random-weight sums are taken modulo it: an entry
/// that differs by any amount below it changes a sum with probability `1 − 1/PRIME`,
/// where modulo 2^64 a difference by a multiple of 2^k would pass with `2^(k−64)`.
pub(crate) const PRIME: u64 = u64::MAX - 58;

/// The table of a partition of a level, and its random-weight sums.
struct Sums {
    table: Table,
    /// `Σ A[u][v]·h(map[v])` over the entries of each community's nodes, modulo
    /// [`PRIME`].
    weighted: Column<u64>,
}

impl Sums {
    fn same(&self, other: &Self) -> bool {
        self.table.internal[..] == other.table.internal[..]
            && self.table.volume[..] == other.table.volume[..]
            && self.weighted[..] == other.weighted[..]
    }
}

/// The random weight of each community of `level`.
fn weights(memory: &Memory, level: u32, count: u32) -> Result<Column<u64>, HierarchyError> {
    Ok(Column::new(memory, "check weights", count as usize, |c| {
        let c = u32::try_from(c).unwrap_or(u32::MAX);
        draw(0, Purpose::HIERARCHY_CHECK, [c, level, 0, 0]) % PRIME
    })?)
}

/// `a + b` modulo [`PRIME`], for `a` and `b` below it.
pub(crate) fn add(a: u64, b: u64) -> u64 {
    let (sum, over) = a.overflowing_add(b);
    if over || sum >= PRIME {
        sum.wrapping_sub(PRIME)
    } else {
        sum
    }
}

/// The sums of the partition `map` (values below `count`) of `source`.
fn sums<S: Source>(
    source: &S,
    memory: &Memory,
    w: Weights<'_>,
    map: &[u32],
    h: &[u64],
) -> Result<Sums, HierarchyError> {
    let count =
        u32::try_from(h.len()).map_err(|_| HierarchyError::Argument("too many communities"))?;
    Ok(Plan::with(source, memory, Settings::default(), |plan| {
        let table = tally(&plan, memory, w, map, count)?;
        let weighted = Column::new(memory, "check sums", h.len(), |_| AtomicU64::new(0))?;
        plan.scan((), all, |(): &mut (), (), _, row, _| {
            let mut sum = 0_u64;
            w.each(&row, |v, x| {
                let term = u128::from(x) * u128::from(h[map[v.index()] as usize]);
                sum = add(sum, u64::try_from(term % u128::from(PRIME)).unwrap_or(0));
            });
            let cell = &weighted[map[row.node.index()] as usize];
            let _ = cell.try_update(Relaxed, Relaxed, |old| Some(add(old, sum)));
        })?;
        let weighted = weighted.map(AtomicU64::into_inner);
        Ok::<_, Error>(Sums { table, weighted })
    })?)
}

/// The sums of `map` over the stored quotient graph of `level`.
fn quotient_sums<S: ByteSource>(
    h: &Hierarchy<'_, S>,
    level: u32,
    memory: &Memory,
    map: &[u32],
    weights: &[u64],
) -> Result<Sums, HierarchyError> {
    let q = h.quotient(level)?;
    let events = q.events()?;
    if events.is_some_and(|e| e.events() >= MAX_EVENTS) {
        return Err(fail(level, "a quotient graph of total weight 2^62 or more"));
    }
    let source = Compressed::new(q.adjacency()?);
    let w = events.map_or(Weights::Unit, Weights::Events);
    sums(&source, memory, w, map, weights)
}

fn levels<S: ByteSource + Sync>(
    file: &Container<S>,
    h: &Hierarchy<'_, S>,
    memory: &Memory,
    report: &mut Report,
) -> Result<(), HierarchyError> {
    let top = h.levels();
    let membership = h.membership(top, memory)?;
    // The smallest node and the node count of every community, from the finest level up.
    let count = h.count(top)? as usize;
    let first = Column::new(memory, "first nodes", count, |_| AtomicU32::new(u32::MAX))?;
    let size = Column::new(memory, "community nodes", count, |_| AtomicU64::new(0))?;
    par::for_each_range(membership.len(), GRAIN, |r| {
        for v in r {
            let c = membership[v] as usize;
            first[c].fetch_min(u32::try_from(v).unwrap_or(u32::MAX), Relaxed);
            size[c].fetch_add(1, Relaxed);
        }
    });
    let mut first = first.map(AtomicU32::into_inner);
    let mut size = size.map(AtomicU64::into_inner);
    for level in (1..=top).rev() {
        let table = h.table(level, memory)?;
        let n = table.nodes.len();
        if (0..n).any(|c| u64::from(table.nodes[c]) != size[c]) {
            return Err(fail(
                level,
                "node counts do not add up from the level below",
            ));
        }
        let ordered = (1..n).all(|c| {
            let (a, b) = (table.nodes[c - 1], table.nodes[c]);
            a > b || (a == b && first[c - 1] < first[c])
        });
        if !ordered || table.nodes.contains(&0) {
            return Err(fail(
                level,
                "communities not numbered by decreasing node count, then smallest node",
            ));
        }
        let h_level = weights(memory, level, h.count(level)?)?;
        let single = Column::new(memory, "communities", n, |c| {
            u32::try_from(c).unwrap_or(u32::MAX)
        })?;
        let own = quotient_sums(h, level, memory, &single, &h_level)?;
        if own.table.internal[..] != table.internal[..] || own.table.volume[..] != table.volume[..]
        {
            return Err(fail(level, "its quotient graph does not hold its table"));
        }
        let made = if level == top {
            let made = graph(file, &membership, memory, &h_level)?;
            if made.is_none() {
                report.unchecked.push(format!(
                    "{OPERATION}: level {top} against the graph (no undirected adjacency.out \
                     in memory)"
                ));
            }
            made
        } else {
            let parents = h.parents(level + 1, memory)?.expect("below level 1");
            Some(quotient_sums(h, level + 1, memory, &parents, &h_level)?)
        };
        if made.is_some_and(|made| !made.same(&own)) {
            return Err(fail(
                level,
                "its quotient graph is not what its partition makes of the level below",
            ));
        }
        if level > 1 {
            let parents = h.parents(level, memory)?.expect("below level 1");
            let above = h.count(level - 1)? as usize;
            let mut up_first = Column::new(memory, "first nodes", above, |_| u32::MAX)?;
            let mut up_size = Column::new(memory, "community nodes", above, |_| 0_u64)?;
            for c in 0..n {
                let p = parents[c] as usize;
                up_first[p] = up_first[p].min(first[c]);
                up_size[p] = up_size[p].saturating_add(u64::from(table.nodes[c]));
            }
            (first, size) = (up_first, up_size);
        }
    }
    Ok(())
}

/// The sums of the membership of the finest level over the graph of the file, when it
/// has an undirected `adjacency.out` in memory.
fn graph<S: ByteSource>(
    file: &Container<S>,
    membership: &[u32],
    memory: &Memory,
    weights: &[u64],
) -> Result<Option<Sums>, HierarchyError> {
    let Some((index, data)) = undirected(file)? else {
        return Ok(None);
    };
    let source = Compressed::new(Adjacency::new(&index, data)?);
    if source.nodes() as usize != membership.len() {
        return Err(HierarchyError::Invalid(
            "a membership of another length than the graph".into(),
        ));
    }
    sums(&source, memory, Weights::Unit, membership, weights).map(Some)
}

/// The undirected `adjacency.out` of the file, when it has one in memory: its index and
/// its data part.
pub(crate) fn undirected<S: ByteSource>(
    file: &Container<S>,
) -> Result<Option<(AdjacencyIndex, &[u8])>, HierarchyError> {
    let slice = |name: &str| {
        let e = file.get(name)?;
        file.source().slice(e.offset..e.offset + e.len)
    };
    let undirected = file
        .provenance("adjacency.out.index")
        .ok()
        .flatten()
        .is_some_and(|p| p.parameters.get("directed").map(String::as_str) == Some("false"));
    let (Some(index), Some(data), true) = (
        slice("adjacency.out.index"),
        slice("adjacency.out.data"),
        undirected,
    ) else {
        return Ok(None);
    };
    Ok(Some((AdjacencyIndex::parse(index)?, data)))
}
