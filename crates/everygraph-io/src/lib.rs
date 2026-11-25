// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native I/O for Everygraph: `ByteSource` backends over files, memory maps and direct
//! I/O, and the sinks that write graphs and spill files. This is the crate where the
//! file system and the operating system are allowed. The backends arrive with issue
//! #29.
//!
//! - [`ProcMemoryProbe`]: the Linux implementation of `everygraph_core::MemoryProbe`,
//!   which reads physical memory and the limits of the process's control group.

#![forbid(unsafe_code)]

mod memory_probe;

pub use memory_probe::ProcMemoryProbe;
