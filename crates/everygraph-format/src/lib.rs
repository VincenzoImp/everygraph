// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `.eg` container (sections, hashes, derived artifacts) and the compressed
//! adjacency codec. It reads through `ByteSource`, so it must build for wasm32. Empty
//! until issues #28 (container) and #30 (codec).

#![forbid(unsafe_code)]
