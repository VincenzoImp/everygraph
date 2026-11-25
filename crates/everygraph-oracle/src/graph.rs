// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The input graph as the oracle sees it: one row per event, before any semantics apply.

use serde::{Deserialize, Serialize};

/// What happens to an edge from a node to itself (`plan/issues/m2-09-graph-semantics.md`,
/// `design/preview/choices.md` A3). Either way the loops are counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfLoops {
    /// The default: dropped from the analytic graph and counted in the ingest report.
    #[default]
    Drop,
    /// Kept. Only the kernels whose meaning with loops is settled run on such graphs
    /// (see `oracle/SEMANTICS.md`).
    Keep,
}

/// The semantics a graph is analysed under. Parallel edges are always kept as events and
/// analysed once per pair, with the multiplicity and the aggregated weight available to
/// weighted kernels (A1); there is no option for them yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Semantics {
    /// Self-loop handling.
    pub self_loops: SelfLoops,
}

/// A graph as a list of events (rows of an edge list), with dense node ids `0..n`.
///
/// Nodes that appear in no row are isolates; `n` is what makes them exist.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    /// A name unique within its corpus; used as a directory name.
    pub name: String,
    /// Number of nodes, isolates included.
    pub n: u32,
    /// Directed or undirected.
    pub directed: bool,
    /// Source of each event.
    pub src: Vec<u32>,
    /// Target of each event.
    pub dst: Vec<u32>,
    /// Optional weight of each event (positive and finite).
    pub weight: Option<Vec<f64>>,
    /// Optional timestamp of each event.
    pub time: Option<Vec<i64>>,
    /// A partition of the nodes (one label per node) for modularity and quotient checks.
    pub partition: Option<Vec<u32>>,
    /// The semantics the graph is analysed under.
    pub semantics: Semantics,
    /// Where the graph came from: generator and seed, or source file and checksum.
    pub provenance: serde_json::Value,
}

impl Graph {
    /// An empty graph with `n` isolated nodes.
    #[must_use]
    pub fn with_nodes(name: &str, n: u32, directed: bool) -> Self {
        Self {
            name: name.to_owned(),
            n,
            directed,
            src: Vec::new(),
            dst: Vec::new(),
            weight: None,
            time: None,
            partition: None,
            semantics: Semantics::default(),
            provenance: serde_json::Value::Null,
        }
    }

    /// Number of events (rows).
    #[must_use]
    pub fn rows(&self) -> usize {
        self.src.len()
    }

    /// Checks that the columns have equal lengths, ids are below `n`, weights are
    /// positive and finite, and the partition has one label per node.
    ///
    /// # Errors
    ///
    /// A description of the first problem found.
    pub fn validate(&self) -> Result<(), String> {
        let rows = self.src.len();
        if self.dst.len() != rows {
            return Err(format!("src has {rows} rows, dst {}", self.dst.len()));
        }
        if let Some(w) = &self.weight {
            if w.len() != rows {
                return Err(format!("weight has {} rows, expected {rows}", w.len()));
            }
            if let Some(i) = w.iter().position(|x| !(x.is_finite() && *x > 0.0)) {
                return Err(format!(
                    "weight of row {i} is {}, not positive and finite",
                    w[i]
                ));
            }
        }
        if let Some(t) = &self.time {
            if t.len() != rows {
                return Err(format!("time has {} rows, expected {rows}", t.len()));
            }
        }
        if let Some(i) = (0..rows).find(|&i| self.src[i] >= self.n || self.dst[i] >= self.n) {
            return Err(format!(
                "row {i} ({}, {}) has an id >= n = {}",
                self.src[i], self.dst[i], self.n
            ));
        }
        if let Some(p) = &self.partition {
            if p.len() != crate::ix(self.n) {
                return Err(format!("partition has {} labels, n = {}", p.len(), self.n));
            }
        }
        Ok(())
    }
}
