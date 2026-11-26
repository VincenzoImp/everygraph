// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The on-disk formats of Everygraph.
//!
//! What lives here:
//!
//! - [`adjacency`]: the compressed adjacency codec (DESIGN D4,
//!   `docs/spec/eg-adjacency.md`), issue #30. An adjacency stores pairs, each neighbour
//!   once; the `.events` section of the specification's revision 2 (ADR 0028) comes with
//!   issue #35.
//! - [`container`]: the `.eg` container (DESIGN D7, `docs/spec/eg-container.md`):
//!   named, BLAKE3-hashed sections behind a table of contents, appended and committed
//!   by a crash-safe protocol over `everygraph_core::ByteSink`, issue #28.
//! - [`verify`]: what `everygraph verify` checks: the container's rules, every section's
//!   hash and every adjacency's structure, issue #41.
//!
//! The crate reads bytes it is given and never opens files, starts threads or reads the
//! clock, so it builds for wasm32; parallel work goes through `everygraph_core::par`.
//! `unsafe` is confined to the SIMD decoders (`adjacency::simd`, registered in
//! CONTRIBUTING.md); everything else is safe code.

// The portability lints cannot be allowed away (see clippy.toml). `unsafe_code` is
// denied by the workspace and allowed in the one registered module.
#![forbid(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

pub mod adjacency;
pub mod container;
pub mod verify;
