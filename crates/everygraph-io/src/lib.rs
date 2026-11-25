// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native I/O for Everygraph: `ByteSource` backends over files, memory maps and direct
//! I/O, and the sinks that write graphs and spill files. This is the crate where the
//! file system and the operating system are allowed. Empty until issue #29.

#![forbid(unsafe_code)]
