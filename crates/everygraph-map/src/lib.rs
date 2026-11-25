// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The map: hierarchy-constrained layout, the `.egmap` tile format and the tile pyramid
//! builder. It must build for wasm32. Empty until issues #67 (layout), #68 (tile
//! format) and #69 (tile builder).

// `forbid`: the allocation rule in `clippy.toml` cannot be allowed away (issue #31).
#![forbid(unsafe_code, clippy::disallowed_methods)]
