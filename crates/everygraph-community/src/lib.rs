// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Community detection: deterministic parallel Leiden, and the hierarchy the map is
//! built from. It must build for wasm32. Empty until issue #57.

// `forbid`: the allocation rule in `clippy.toml` cannot be allowed away (issue #31).
#![forbid(unsafe_code, clippy::disallowed_methods)]
