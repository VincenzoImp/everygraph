// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What can be asked of a kernel, and the canonical form of its answer.
//!
//! The JSON form of both types is the exchange format with the Python references and with
//! external kernels (`oracle/README.md`, "Exchange formats"): a kernel is an object tagged
//! by `"kernel"`, an output an object with exactly one key naming its shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A kernel and its parameters. Every kernel runs on the analytic graph of the input
/// (the projection in [`crate::projection`]): self-loops dropped unless kept, parallel
/// edges analysed once per pair. Kernels marked "undirected projection" ignore direction.
/// A kept loop is the pair `{v, v}` (`docs/spec/graph-semantics.md` § 5): volume and flow
/// measures count it as `A[v][v] = 2w` undirected, `w` directed; core numbers, triangles,
/// clustering, transitivity, components and traversals ignore it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kernel", rename_all = "snake_case")]
pub enum Kernel {
    /// The out-adjacency of the analytic graph as sorted, duplicate-free lists
    /// (symmetric for undirected graphs); a node with a kept loop lists itself once.
    /// Output: [`Output::Csr`].
    Adjacency,
    /// The ingest report of `docs/spec/graph-semantics.md` § 2.8, keyed as printed
    /// (`counts.rows`, `dropped.self_loops`, ...). Output: [`Output::Counts`].
    IngestReport,
    /// Number of distinct neighbours; a kept loop adds 2 (repeated loop rows are one loop).
    /// On a directed graph, distinct neighbours in either direction: the degree in the
    /// undirected projection, so a reciprocal pair is one neighbour (Q10).
    /// Output: [`Output::Ints`].
    Degree,
    /// Directed graphs: distinct out-neighbours, a kept loop adds 1.
    DegreeOut,
    /// Directed graphs: distinct in-neighbours, a kept loop adds 1.
    DegreeIn,
    /// PageRank at its fixed point: uniform teleport, dangling mass spread uniformly,
    /// out-edges for directed graphs, a kept loop as `A[v][v]` (S1). `weighted` uses the
    /// summed event weight of each pair (the multiplicity when the graph has no weight
    /// column). With seeds, teleport and dangling mass are uniform over the seeds.
    /// Output: [`Output::Reals`].
    #[serde(rename = "pagerank")]
    PageRank {
        /// Damping factor (probability of following an edge).
        damping: f64,
        /// Weight transitions by the pair's summed weight.
        weighted: bool,
        /// The seeds of a personalized PageRank (distinct, increasing); empty: every node.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        personalize: Vec<u32>,
    },
    /// Eigenvector centrality: the eigenvector of the largest eigenvalue of `Aᵀ` (in-edges
    /// on a directed graph, a kept loop as `A[v][v]` as for PageRank), with unit L2 norm
    /// and no negative value. Listed only where that eigenvector is unique: a connected
    /// (strongly connected, if directed) graph with an edge. `weighted` as for PageRank.
    /// Output: [`Output::Reals`].
    Eigenvector {
        /// Weight the matrix by the pair's summed weight.
        weighted: bool,
    },
    /// Weakly connected components; label = smallest node id of the component.
    /// Output: [`Output::Labels`].
    Wcc,
    /// Strongly connected components (directed graphs); label = smallest node id.
    Scc,
    /// Hop distances from `source` along out-edges; unreachable is `None`.
    /// Output: [`Output::Ints`].
    Bfs {
        /// Source node.
        source: u32,
    },
    /// Weighted distances from `source` along out-edges, using the smallest event weight
    /// of each pair (1 when the graph has no weight column). Output: [`Output::Reals`].
    Sssp {
        /// Source node.
        source: u32,
    },
    /// Core numbers of the undirected projection. Output: [`Output::Ints`].
    CoreNumber,
    /// Triangles through each node, undirected projection. Output: [`Output::Ints`].
    Triangles,
    /// Number of triangles, undirected projection. Output: [`Output::Count`].
    TriangleTotal,
    /// Local clustering coefficient, undirected projection; 0 where degree < 2.
    /// Output: [`Output::Reals`].
    Clustering,
    /// Global clustering (transitivity), undirected projection; 0 without any connected
    /// triple. Output: [`Output::Real`].
    Transitivity,
    /// Newman modularity of the graph's partition, undirected projection, a kept loop a
    /// pair inside its node's community (S5); 0 for a graph without edges.
    /// Output: [`Output::Real`].
    Modularity {
        /// Resolution γ.
        resolution: f64,
        /// Weight pairs by their summed event weight (the multiplicity without a weight
        /// column); unweighted (1 per pair) otherwise.
        #[serde(default)]
        weighted: bool,
    },
    /// Quotient graph of the graph's partition, undirected projection, unweighted: the
    /// size of every community and the number of pairs between (or inside) communities,
    /// kept loops included on the diagonal (S6). Output: [`Output::Quotient`].
    Quotient,
}

impl Kernel {
    /// The name used in reports and in the `"kernel"` tag.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Adjacency => "adjacency",
            Self::IngestReport => "ingest_report",
            Self::Degree => "degree",
            Self::DegreeOut => "degree_out",
            Self::DegreeIn => "degree_in",
            Self::PageRank { .. } => "pagerank",
            Self::Eigenvector { .. } => "eigenvector",
            Self::Wcc => "wcc",
            Self::Scc => "scc",
            Self::Bfs { .. } => "bfs",
            Self::Sssp { .. } => "sssp",
            Self::CoreNumber => "core_number",
            Self::Triangles => "triangles",
            Self::TriangleTotal => "triangle_total",
            Self::Clustering => "clustering",
            Self::Transitivity => "transitivity",
            Self::Modularity { .. } => "modularity",
            Self::Quotient => "quotient",
        }
    }

    /// The kernel with its parameters, compactly, for reports.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::PageRank {
                damping,
                weighted,
                personalize,
            } => {
                let seeds = if personalize.is_empty() {
                    String::new()
                } else {
                    format!(",seeds={personalize:?}")
                };
                format!(
                    "pagerank(d={damping}{}{seeds})",
                    if *weighted { ",weighted" } else { "" }
                )
            }
            Self::Eigenvector { weighted } => {
                format!("eigenvector({})", if *weighted { "weighted" } else { "" })
            }
            Self::Bfs { source } => format!("bfs(source={source})"),
            Self::Sssp { source } => format!("sssp(source={source})"),
            Self::Modularity {
                resolution,
                weighted,
            } => format!(
                "modularity(γ={resolution}{})",
                if *weighted { ",weighted" } else { "" }
            ),
            other => other.name().to_owned(),
        }
    }
}

/// One weighted edge of a quotient graph: `(community, community, weight)` with the first
/// label not larger than the second.
pub type QuotientEdge = (u32, u32, f64);

/// The canonical form of a kernel's answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    /// One integer per node; `None` where there is no value (unreachable).
    Ints(Vec<Option<u64>>),
    /// One real per node; `None` where there is no value (unreachable).
    Reals(Vec<Option<f64>>),
    /// One label per node. Compared after canonical relabelling, so any labelling of the
    /// same classes is equal; the kernels above also promise "smallest node id", which
    /// an invariant checks separately.
    Labels(Vec<u32>),
    /// A single count.
    Count(u64),
    /// A single real.
    Real(f64),
    /// Adjacency lists in compressed sparse row form.
    Csr {
        /// `n + 1` offsets into `targets`.
        offsets: Vec<u64>,
        /// Concatenated neighbour lists.
        targets: Vec<u32>,
    },
    /// Named counts.
    Counts(BTreeMap<String, u64>),
    /// A quotient graph.
    Quotient {
        /// Number of nodes in each community, indexed by community label.
        sizes: Vec<u64>,
        /// Weighted edges between communities, sorted, each pair once.
        edges: Vec<QuotientEdge>,
    },
}

impl Output {
    /// The name of the output's shape, as in its JSON key.
    #[must_use]
    pub const fn shape(&self) -> &'static str {
        match self {
            Self::Ints(_) => "ints",
            Self::Reals(_) => "reals",
            Self::Labels(_) => "labels",
            Self::Count(_) => "count",
            Self::Real(_) => "real",
            Self::Csr { .. } => "csr",
            Self::Counts(_) => "counts",
            Self::Quotient { .. } => "quotient",
        }
    }
}
