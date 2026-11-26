// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The map: hierarchy-constrained layout, the `.egmap` tile format and the tile pyramid
//! builder. It must build for wasm32. Today it holds the file format, [`egmap`]
//! (`docs/spec/egmap.md`, issue #68); the layout (#67) and the builder (#69) follow.

// `forbid`: the allocation and portability rules in `clippy.toml` cannot be allowed
// away (issue #31).
#![forbid(
    unsafe_code,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

pub mod egmap;
