// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Whole-graph operations by name, as the command line and the Python API ask for them
//! (`docs/api-sketch.md` § 3.2 and § 5.1): each one's name, version and parameters,
//! which make the key a stored result is found by (`docs/spec/eg-container.md` § 6),
//! and the kernel it runs on the out-lists of a graph file.
//!
//! [`stored`] finds a result in the graph file before it is computed, and [`store`]
//! keeps one there (`docs/api-sketch.md` § 2.8); `bfs` is never kept.
//!
//! A graph file holds the out-adjacency only; an undirected graph's out-lists are its
//! lists. On a directed graph, `pagerank` and `kcore` need the in-adjacency or the
//! undirected projection, which no file holds yet (#m2-11): they are
//! [`RunError::Unsupported`] there. `components` reads the out-lists of a directed
//! graph as weak components do, and `bfs` follows them out.

use std::fmt;
use std::sync::atomic::AtomicU32;

use everygraph_core::{ByteSink, ByteSource, Memory, MemoryError, SegVec};
use everygraph_format::container::{
    Container, ContainerError, GraphIdentity, Provenance, ResultKey, StoredResult, Value,
    ValueType, Writer,
};

use crate::components::{self, WccOptions};
use crate::degrees::{self, Loop};
use crate::edge_map::{Error, Plan, Source};
use crate::kcore;
use crate::pagerank::{self, PageRank};
use crate::traversal::{self, Bfs, UNREACHED};

/// The state precision of `pagerank` (`docs/api-sketch.md` § 2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    /// f64.
    Full,
    /// f32, only when asked for.
    Low,
}

/// A whole-graph operation and its parameters.
#[derive(Clone, Copy, Debug)]
pub enum Operation {
    /// `PageRank`.
    PageRank {
        /// Damping, iterations at most, tolerance; no personalization.
        params: PageRank<'static>,
        /// The state's precision.
        precision: Precision,
    },
    /// Weakly connected components, numbered by decreasing size.
    Components,
    /// Core numbers.
    KCore,
    /// The degree of each node: distinct neighbours.
    Degrees,
    /// Hops from one node.
    Bfs {
        /// The source node.
        source: u32,
        /// Stop at this distance.
        max_hops: Option<u32>,
    },
}

/// The names, in the order the help lists them.
pub const NAMES: [&str; 5] = ["pagerank", "components", "kcore", "degrees", "bfs"];

impl Operation {
    /// The name, as typed after `everygraph run`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::PageRank { .. } => "pagerank",
            Self::Components => "components",
            Self::KCore => "kcore",
            Self::Degrees => "degrees",
            Self::Bfs { .. } => "bfs",
        }
    }

    /// The algorithm's version: a stored result of another version is not served.
    #[must_use]
    pub fn version(&self) -> u32 {
        1
    }

    /// The version of the operation named `name`, if there is one.
    #[must_use]
    pub fn version_of(name: &str) -> Option<u32> {
        NAMES.contains(&name).then_some(1)
    }

    /// The name of the column it computes, the first for `bfs` (`docs/api-sketch.md`
    /// § 3.2): `degrees` of a directed graph gives `out_degree`.
    #[must_use]
    pub fn column(&self, directed: bool) -> &'static str {
        match self {
            Self::PageRank { .. } => "pagerank",
            Self::Components => "component",
            Self::KCore => "core",
            Self::Degrees if directed => "out_degree",
            Self::Degrees => "degree",
            Self::Bfs { .. } => "distance",
        }
    }

    /// Whether its result on the whole graph is kept in the graph file: all but `bfs`,
    /// which is one table per source (`docs/api-sketch.md` § 2.8).
    #[must_use]
    pub fn stored(&self) -> bool {
        !matches!(self, Self::Bfs { .. })
    }

    /// The key its result on the graph `graph` is kept under: every run is
    /// deterministic.
    ///
    /// # Errors
    ///
    /// As [`ResultKey::with`], for a parameter the key cannot hold.
    pub fn key(&self, graph: GraphIdentity) -> Result<ResultKey, ContainerError> {
        self.parameters().into_iter().try_fold(
            ResultKey::new(graph, self.name(), self.version(), true)?,
            |key, (name, value)| key.with(name, &value),
        )
    }

    /// Every parameter that can change the result, with its value as the key holds it
    /// (spec § 6: floating-point values as Rust's `{}` writes them).
    #[must_use]
    pub fn parameters(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::PageRank { params, precision } => vec![
                ("damping", params.damping.to_string()),
                ("max_iter", params.max_iter.to_string()),
                (
                    "precision",
                    match precision {
                        Precision::Full => "full",
                        Precision::Low => "low",
                    }
                    .to_owned(),
                ),
                ("tol", params.tol.to_string()),
            ],
            Self::Bfs { source, max_hops } => {
                let mut p = vec![("source", source.to_string())];
                p.extend(max_hops.map(|h| ("max_hops", h.to_string())));
                p
            }
            Self::Components | Self::KCore | Self::Degrees => Vec::new(),
        }
    }
}

/// One column of a result, one value per node in node order.
#[derive(Debug)]
pub enum Column {
    /// Integers; `null` is the value that stands for none (`bfs` where a node is not
    /// reached).
    U32 {
        /// The values.
        values: SegVec<u32>,
        /// The value that means null, if any.
        null: Option<u32>,
    },
    /// Nodes, by their number; `null` stands for none.
    Nodes {
        /// The nodes.
        values: SegVec<u32>,
        /// The value that means null.
        null: u32,
    },
    /// f64 values.
    F64(SegVec<f64>),
    /// f32 values.
    F32(SegVec<f32>),
}

/// What an operation computed.
#[derive(Debug)]
pub struct Output {
    /// The columns after `node`, by name (`docs/api-sketch.md` § 3.2).
    pub columns: Vec<(&'static str, Column)>,
    /// Iterations run, for an iteration.
    pub iterations: Option<u32>,
    /// Whether it stopped below its tolerance, for an iteration.
    pub converged: Option<bool>,
}

/// Why an operation did not run.
#[derive(Debug)]
pub enum RunError {
    /// The kernel failed.
    Kernel(Error),
    /// The operation cannot run on this graph; the reason.
    Unsupported(&'static str),
    /// A stored result could not be read: the file is damaged.
    Stored(ContainerError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kernel(e) => e.fmt(f),
            Self::Unsupported(why) => f.write_str(why),
            Self::Stored(e) => e.fmt(f),
        }
    }
}

impl From<ContainerError> for RunError {
    fn from(e: ContainerError) -> Self {
        Self::Stored(e)
    }
}

impl std::error::Error for RunError {}

impl From<Error> for RunError {
    fn from(e: Error) -> Self {
        Self::Kernel(e)
    }
}

impl From<MemoryError> for RunError {
    fn from(e: MemoryError) -> Self {
        Self::Kernel(Error::Memory(e))
    }
}

const NEEDS_IN_LISTS: &str =
    "on a directed graph it needs the in-adjacency, which this file does not hold";

/// Runs `op` on the out-lists that `out` reads, of a graph that is `directed` or not,
/// within `memory`.
///
/// # Errors
///
/// [`RunError::Unsupported`] for `pagerank` and `kcore` on a directed graph; the
/// kernel's errors.
pub fn run<S: Source>(
    op: &Operation,
    out: &Plan<'_, '_, S>,
    directed: bool,
    memory: &Memory,
) -> Result<Output, RunError> {
    let plain = |name, column| Output {
        columns: vec![(name, column)],
        iterations: None,
        converged: None,
    };
    let labels = |values| Column::U32 { values, null: None };
    match *op {
        Operation::PageRank { params, precision } => {
            if directed {
                return Err(RunError::Unsupported(NEEDS_IN_LISTS));
            }
            let g = pagerank::Graph {
                lists: out,
                weights: None,
                directed,
            };
            let strength = pagerank::strength(out, None, directed, memory)?;
            let strength = strength.as_slice().ok_or(Error::Invalid(
                "node state longer than one segment (#m3-15)",
            ))?;
            let (column, iterations, change) = match precision {
                Precision::Full => {
                    let r = pagerank::pagerank::<_, f64>(&g, strength, &params, memory)?;
                    (Column::F64(r.values), r.iterations, r.change)
                }
                Precision::Low => {
                    let r = pagerank::pagerank::<_, f32>(&g, strength, &params, memory)?;
                    (Column::F32(r.values), r.iterations, r.change)
                }
            };
            Ok(Output {
                iterations: Some(iterations),
                converged: Some(change < params.tol),
                ..plain("pagerank", column)
            })
        }
        Operation::Components => {
            let options = WccOptions {
                symmetric: !directed,
                sample: true,
            };
            let (found, _held) = components::wcc(out, memory, options)?;
            let numbered = components::number(&found, memory)?;
            Ok(plain("component", labels(numbered.component)))
        }
        Operation::KCore => {
            if directed {
                return Err(RunError::Unsupported(NEEDS_IN_LISTS));
            }
            let (core, _held) = kcore::kcore(out, memory)?;
            Ok(plain("core", labels(to_column(&core, memory)?)))
        }
        Operation::Degrees => {
            let own = if directed { Loop::Once } else { Loop::Twice };
            let values = degrees::degrees(out, memory, own)?;
            Ok(plain(op.column(directed), labels(values)))
        }
        Operation::Bfs { source, max_hops } => {
            let p = Bfs {
                max_hops,
                parents: true,
            };
            let inn = (!directed).then_some(out);
            let hops = traversal::bfs(out, inn, source, &p, memory)?;
            let mut columns = vec![(
                "distance",
                Column::U32 {
                    values: to_column(&hops.distance, memory)?,
                    null: Some(UNREACHED),
                },
            )];
            if let Some(parent) = &hops.parent {
                columns.push((
                    "parent",
                    Column::Nodes {
                        values: to_column(parent, memory)?,
                        null: UNREACHED,
                    },
                ));
            }
            Ok(Output {
                columns,
                iterations: None,
                converged: None,
            })
        }
    }
}

/// The result of `op` on the graph of `file`, a graph that is `directed` or not, if
/// the file keeps one: checked against its hash and read into `memory`.
///
/// # Errors
///
/// [`RunError::Stored`] for a result that is damaged; [`RunError::Kernel`] with
/// [`Error::Memory`] when its values do not fit the budget.
pub fn stored<S: ByteSource>(
    file: &Container<S>,
    op: &Operation,
    directed: bool,
    memory: &Memory,
) -> Result<Option<Output>, RunError> {
    if !op.stored() {
        return Ok(None);
    }
    let Some(found) = file.find_result(&op.key(file.graph_identity())?)? else {
        return Ok(None);
    };
    file.verify_result(&found)?;
    let len = usize::try_from(found.len())
        .map_err(|_| Error::Invalid("a result longer than the address space"))?;
    let column = match found.value_type() {
        ValueType::U32 => Column::U32 {
            values: read(&found, len, memory)?,
            null: None,
        },
        ValueType::F64 => Column::F64(read(&found, len, memory)?),
        ValueType::F32 => Column::F32(read(&found, len, memory)?),
        _ => return Ok(None),
    };
    Ok(Some(Output {
        columns: vec![(op.column(directed), column)],
        iterations: None,
        converged: None,
    }))
}

fn read<S: ByteSource, T: Value + Default>(
    found: &StoredResult<'_, S>,
    len: usize,
    memory: &Memory,
) -> Result<SegVec<T>, RunError> {
    let mut values = SegVec::try_with_len(memory, "stored result", len, T::default())?;
    let mut first = 0;
    for segment in values.segments_mut() {
        found.read(first, segment)?;
        first += segment.len() as u64;
    }
    Ok(values)
}

/// Adds the first column of `output` to `writer` as the result of `op` on its graph,
/// with `about` for the engine and the times; [`Writer::commit`] makes it stored.
///
/// # Errors
///
/// The errors of [`Writer::add_result`]; [`ContainerError::InvalidResult`] for a
/// column longer than one segment.
pub fn store<K: ByteSink>(
    writer: &mut Writer<K>,
    op: &Operation,
    about: &Provenance,
    output: &Output,
) -> Result<(), ContainerError> {
    let key = op.key(writer.graph_identity())?;
    let Some((_, column)) = output.columns.first() else {
        return Ok(());
    };
    let long = ContainerError::InvalidResult("a result longer than one segment (#m3-15)");
    match column {
        Column::U32 { values, .. } | Column::Nodes { values, .. } => {
            writer.add_result(&key, about, values.as_slice().ok_or(long)?)
        }
        Column::F64(values) => writer.add_result(&key, about, values.as_slice().ok_or(long)?),
        Column::F32(values) => writer.add_result(&key, about, values.as_slice().ok_or(long)?),
    }
    .map(|_| ())
}

/// The values of `atomics`, in a column of the budget.
fn to_column(atomics: &[AtomicU32], memory: &Memory) -> Result<SegVec<u32>, RunError> {
    let mut out = SegVec::try_with_len(memory, "result column", atomics.len(), 0_u32)?;
    for (slot, a) in out.segments_mut().flatten().zip(atomics) {
        *slot = a.load(std::sync::atomic::Ordering::Relaxed);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use everygraph_core::MemorySource;

    use super::*;
    use crate::edge_map::{Csr, Settings};

    fn values(output: &Output) -> &[u32] {
        match &output.columns[0].1 {
            Column::U32 { values, .. } => values.as_slice().unwrap(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_stored_result_is_found_under_its_own_operation_only() {
        // The triangle 0 1 2 and the edge 2 3, as symmetric lists.
        let (offsets, targets) = ([0, 2, 4, 7, 8], [1, 2, 0, 2, 0, 1, 3, 2]);
        let csr = Csr::new(&offsets, &targets).unwrap();
        let memory = Memory::unlimited();
        let output = Plan::with(&csr, &memory, Settings::default(), |plan| {
            run(&Operation::Degrees, &plan, false, &memory)
        })
        .unwrap();
        assert_eq!(values(&output), [2, 2, 3, 1]);

        let mut writer = Writer::create(MemorySource::new(Vec::new())).unwrap();
        store(
            &mut writer,
            &Operation::Degrees,
            &Provenance::default(),
            &output,
        )
        .unwrap();
        writer.commit().unwrap();
        let file = Container::open(writer.into_sink()).unwrap();
        let found = stored(&file, &Operation::Degrees, false, &memory)
            .unwrap()
            .unwrap();
        assert_eq!(found.columns[0].0, "degree");
        assert_eq!(values(&found), [2, 2, 3, 1]);
        for other in [
            Operation::KCore,
            Operation::Bfs {
                source: 0,
                max_hops: None,
            },
        ] {
            assert!(stored(&file, &other, false, &memory).unwrap().is_none());
        }
        let damped = |damping| Operation::PageRank {
            params: PageRank {
                damping,
                ..PageRank::default()
            },
            precision: Precision::Full,
        };
        assert_ne!(
            damped(0.85).key(file.graph_identity()).unwrap(),
            damped(0.5).key(file.graph_identity()).unwrap()
        );
    }
}
