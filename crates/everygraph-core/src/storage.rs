// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`Storage`]: what holds a file, as the planner needs to know it (`docs/ux-contract.md`
//! P-2). The core defines it and never probes it: `everygraph_io::storage` reads it on
//! Linux.

use std::fmt;

/// The kind of device or file system that holds a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorageKind {
    /// A local `NVMe` drive.
    Nvme,
    /// Another local solid-state drive.
    Ssd,
    /// A local rotating disk.
    Hdd,
    /// A file system held in memory (`tmpfs`, `ramfs`): its files are memory (P-18).
    Memory,
    /// A network or user-space file system (NFS, SMB, FUSE). Its files are never
    /// memory-mapped (`design/DECISIONS.md`, "Memory maps and I/O errors").
    Remote,
    /// Not known.
    Unknown,
}

impl fmt::Display for StorageKind {
    /// The `machine.storage.kind` of a plan document, as the `machine` line prints it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Nvme => "NVMe",
            Self::Ssd => "SSD",
            Self::Hdd => "HDD",
            Self::Memory => "memory",
            Self::Remote => "network",
            Self::Unknown => "unknown",
        })
    }
}

/// The storage of a directory: its kind, and the bytes free there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Storage {
    /// The kind of device.
    pub kind: StorageKind,
    /// Bytes a process can still write there.
    pub free_bytes: u64,
}
