// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`check`]: the conservation checks of `everygraph verify --deep` on every stored
//! hierarchy (`docs/spec/eg-communities.md` § 3).

use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicU32, AtomicU64};

use everygraph_core::{ByteSource, Memory, ReadError, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex};
use everygraph_format::container::{Container, ContainerError};
use everygraph_format::verify::Report;
use everygraph_kernels::edge_map::{Compressed, Error, Plan, Settings, Source};

use super::{Hierarchy, HierarchyError, OPERATION};
use crate::column::{Column, GRAIN};
use crate::level::Weights;
use crate::quality::{Table, tally};

/// Checks every hierarchy stored for the graph of `file` (held in memory: a memory
/// map), and adds what is wrong to `report`:
///
/// - at every level, node counts that add up to the graph's nodes and to those of the
///   level above, and communities numbered by decreasing node count, then smallest node;
/// - the table of every level equal to that of its quotient graph under singletons, and
///   to that of the partition its parents make of the level below; the finest level's
///   to that of the membership of the graph (`adjacency.out`, when undirected). So the
///   volumes add up to `2W` and the internal and cut weights to `W` at every level.
///
/// What the budget does not hold is listed as unchecked.
///
/// # Errors
///
/// [`ReadError::Backend`] when the file cannot be read.
pub fn check<S: ByteSource + Sync>(
    file: &Container<S>,
    memory: &Memory,
    report: &mut Report,
) -> Result<(), ReadError> {
    let Ok(listed) = file.results() else {
        return Ok(());
    };
    for result in listed
        .iter()
        .filter(|r| r.provenance.operation == OPERATION && r.of_this_graph)
    {
        let outcome = file
            .listed_result(result)
            .map_err(HierarchyError::File)
            .and_then(|root| Hierarchy::of(file, root))
            .and_then(|h| levels(file, &h, memory, report));
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

/// Whether two tables hold the same internal weights and volumes.
fn same(a: &Table, b: &Table) -> bool {
    a.internal[..] == b.internal[..] && a.volume[..] == b.volume[..]
}

/// The table of `map` (values below `count`) over the stored quotient of `level`.
fn quotient_table<S: ByteSource>(
    h: &Hierarchy<'_, S>,
    level: u32,
    memory: &Memory,
    map: &[u32],
    count: u32,
) -> Result<Table, HierarchyError> {
    let q = h.quotient(level)?;
    let events = q.events()?;
    let source = Compressed::new(q.adjacency()?);
    let w = events.map_or(Weights::Unit, Weights::Events);
    Ok(Plan::with(&source, memory, Settings::default(), |plan| {
        tally(&plan, memory, w, map, count)
    })?)
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
            first[c].fetch_min(u32::try_from(v).expect("a node"), Relaxed);
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
        let single = Column::new(memory, "communities", n, |c| {
            u32::try_from(c).expect("a community")
        })?;
        if !same(
            &quotient_table(h, level, memory, &single, h.count(level)?)?,
            &table,
        ) {
            return Err(fail(level, "its quotient graph does not hold its table"));
        }
        if level == top {
            graph(
                file,
                &membership,
                &table,
                memory,
                report,
                top,
                h.count(top)?,
            )?;
        } else {
            let parents = h.parents(level + 1, memory)?.expect("below level 1");
            let made = quotient_table(h, level + 1, memory, &parents, h.count(level)?)?;
            if !same(&made, &table) {
                return Err(fail(
                    level,
                    "its table is not that of its parents' partition",
                ));
            }
        }
        if level > 1 {
            let parents = h.parents(level, memory)?.expect("below level 1");
            let above = h.count(level - 1)? as usize;
            let mut up_first = Column::new(memory, "first nodes", above, |_| u32::MAX)?;
            let mut up_size = Column::new(memory, "community nodes", above, |_| 0_u64)?;
            for c in 0..n {
                let p = parents[c] as usize;
                up_first[p] = up_first[p].min(first[c]);
                up_size[p] += u64::from(table.nodes[c]);
            }
            (first, size) = (up_first, up_size);
        }
    }
    Ok(())
}

/// The finest level's table against the membership of the graph of the file, when it
/// has an undirected `adjacency.out`.
fn graph<S: ByteSource>(
    file: &Container<S>,
    membership: &[u32],
    table: &Table,
    memory: &Memory,
    report: &mut Report,
    top: u32,
    count: u32,
) -> Result<(), HierarchyError> {
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
        report.unchecked.push(format!(
            "{OPERATION}: level {top} against the graph (no undirected adjacency.out in memory)"
        ));
        return Ok(());
    };
    let index = AdjacencyIndex::parse(index)?;
    let source = Compressed::new(Adjacency::new(&index, data)?);
    if source.nodes() as usize != membership.len() {
        return Err(fail(top, "a membership of another length than the graph"));
    }
    let made = Plan::with(&source, memory, Settings::default(), |plan| {
        tally(&plan, memory, Weights::Unit, membership, count)
    })?;
    if !same(&made, table) {
        return Err(fail(top, "its table is not that of the graph's partition"));
    }
    Ok(())
}
