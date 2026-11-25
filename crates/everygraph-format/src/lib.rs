// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The on-disk formats of Everygraph.
//!
//! What lives here:
//!
//! - [`adjacency`]: the compressed adjacency codec (DESIGN D4,
//!   `docs/spec/eg-adjacency.md`), issue #30.
//! - The `.eg` container (sections, hashes, derived artifacts): issue #28, not yet.
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
