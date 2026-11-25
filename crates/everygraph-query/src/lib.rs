// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Queries: neighbours and k-hop with limits, shortest paths, and filtered views. Empty
//! until issues #90 (point queries), #91 (shortest paths) and #93 (views).

// `forbid`: the allocation rule in `clippy.toml` cannot be allowed away (issue #31).
#![forbid(unsafe_code, clippy::disallowed_methods)]
