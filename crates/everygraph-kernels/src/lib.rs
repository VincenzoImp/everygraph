// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The edge-map framework and the kernels written on it once for every execution regime
//! (`PageRank`, components, traversals, k-core, triangles, centrality).
//!
//! What lives here:
//!
//! - [`edge_map`]: the framework: the adjacency [`Source`](edge_map::Source)s (raw
//!   arrays, the compressed format in the heap or memory-mapped, streams), the
//!   [`Plan`](edge_map::Plan) whose drivers run a kernel body over them, and the rules
//!   that keep every result independent of threads, chunking and source.
//! - [`pagerank`]: `PageRank` (f32 and f64 state, weighted, personalized) and
//!   eigenvector centrality, by pull power iteration (#47).
//! - [`traversal`]: direction-optimizing BFS and bit-parallel multi-source BFS (#49).
//! - [`sssp`]: weighted shortest distances by Δ*-stepping with bucket fusion (#49).
//! - [`components`]: weakly connected components and their numbering (#48).
//! - [`scc`]: strongly connected components of a directed graph, by BGSS (#223).
//! - [`kcore`]: core numbers and shells (#50).
//! - [`degrees`]: degree and strength columns (#50).
//! - [`proof`]: the lab's Afforest on the framework, the first-neighbour baseline of
//!   [`components`].
//! - [`run`]: the whole-graph operations by name, as the command line and Python ask
//!   for them, and their results kept in the graph file.
//!
//! A kernel body is a closure over one node and its list:
//!
//! ```
//! use everygraph_core::Memory;
//! use everygraph_kernels::edge_map::{Csr, Plan, Settings, all};
//!
//! // Node 0 lists 1 and 2; node 1 lists 0; node 2 lists 0.
//! let (offsets, targets) = ([0, 2, 3, 4], [1, 2, 0, 0]);
//! let csr = Csr::new(&offsets, &targets).unwrap();
//! let weight = [1.0_f64, 10.0, 100.0];
//!
//! let mut heaviest = [0_u32; 3];
//! let total: f64 = Plan::with(&csr, &Memory::unlimited(), Settings::default(), |plan| {
//!     let w = plan.per_node(&weight).unwrap(); // indexed by nodes, without bounds checks
//!     // For each node: its heaviest neighbour (an output), and the sum of the weights
//!     // of the neighbours of all nodes (a reduction, in a fixed order).
//!     plan.scan(&mut heaviest[..], all, |sum: &mut f64, out, i, row, _| {
//!         let best = row.list.iter().max_by(|a, b| w[**a].total_cmp(&w[**b]));
//!         out[i] = best.map_or(0, |u| u.get());
//!         *sum = row.list.iter().fold(*sum, |s, &u| s + w[u]);
//!     })
//! })
//! .unwrap();
//! assert_eq!((heaviest, total), ([2, 0, 0], 112.0));
//! ```
//!
//! The same closure runs over a [`Compressed`](edge_map::Compressed), a
//! [`Stream`](edge_map::Stream) or a [`CompressedStream`](edge_map::CompressedStream)
//! source with the same result. The crate builds for
//! wasm32: it reads through `ByteSource`, splits work through `everygraph_core::par`, and
//! is under the portability lists of the core and the allocation rule (`clippy.toml`).

// `forbid`: the portability and allocation lists in `clippy.toml` cannot be allowed
// away. `unsafe_code` is denied by the workspace and allowed in the one registered
// module, `edge_map` (CONTRIBUTING.md, "`unsafe`").
#![forbid(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use std::sync::atomic::AtomicU32;

use everygraph_core::Reservation;

pub mod components;
pub mod degrees;
pub mod edge_map;
pub mod kcore;
pub mod pagerank;
pub mod proof;
pub mod run;
pub mod scc;
pub mod sssp;
pub mod traversal;
mod weights;

/// Per-node values in atomics, as a kernel computed them, and the reservation that
/// holds them.
pub type Atomics = (Vec<AtomicU32>, Reservation);
