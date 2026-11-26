// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Community detection (DESIGN D14–D16): Leiden on the edge-map framework, the
//! community table and modularity, and the quotient graph of a partition.
//!
//! - [`leiden`]: the communities of an undirected graph given as any
//!   [`Source`](everygraph_kernels::edge_map::Source): raw arrays, the compressed
//!   format in the heap or memory-mapped, a stream. Deterministic by default: the same
//!   input and seed give the same result at any thread count, chunking and source.
//! - [`table`] and [`modularity`]: the community table of any partition and its
//!   modularity, with kept loops as `docs/spec/graph-semantics.md` § 5.5 counts them.
//! - [`quotient_of`]: the quotient graph of a partition, whose modularity under singletons
//!   equals the partition's.
//!
//! In memory only: level 0 is read through the source, coarser levels are held in RAM
//! (out of core: #60). The crate builds for wasm32 and has no `unsafe`; it is under
//! the portability lists of the core and the allocation rule (`clippy.toml`).
//!
//! ```
//! use everygraph_community::{Options, leiden};
//! use everygraph_core::Memory;
//! use everygraph_kernels::edge_map::{Csr, Settings};
//!
//! // Two triangles, 0-1-2 and 3-4-5, joined by the edge 2-3.
//! let offsets = [0, 2, 4, 7, 10, 12, 14];
//! let targets = [1, 2, 0, 2, 0, 1, 3, 2, 4, 5, 3, 5, 3, 4];
//! let csr = Csr::new(&offsets, &targets).unwrap();
//! let c = leiden(&csr, &Memory::unlimited(), Settings::default(), &Options::default()).unwrap();
//! assert_eq!(&c.membership[..], &[0, 0, 0, 1, 1, 1]);
//! assert!((c.modularity - 5.0 / 14.0).abs() < 1e-12);
//! ```

#![forbid(
    unsafe_code,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

mod aggregate;
mod column;
mod leiden;
mod level;
mod quality;
mod table;

pub use aggregate::{Quotient, quotient_of};
pub use column::Column;
pub use leiden::{Communities, Iterations, LevelStats, Mode, Options, Refinement, SplitAt, leiden};
pub use quality::{Table, modularity, table};
