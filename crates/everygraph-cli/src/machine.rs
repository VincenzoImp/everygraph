// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the machine has (`docs/ux-contract.md` P-2), the memory budget an operation
//! gets from it (P-11, P-16), and the accountant that holds the operation to it.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;

use everygraph_core::{Budget, Memory, MemoryProbe, SystemMemory};
use everygraph_io::ProcMemoryProbe;

use crate::args::Common;
use crate::failure::{self, Failure};
use crate::render::{rough_size, size, typed};

/// `statfs` types of file systems held in memory (Linux `magic.h`).
#[cfg(target_os = "linux")]
const MEMORY_FILE_SYSTEMS: [u64; 2] = [0x0102_1994, 0x8584_58f6];
/// The budget when the machine's free memory is not known.
const UNKNOWN_FREE_BUDGET: u64 = 8_000_000_000;

/// The accountant of an operation under a budget of `bytes`.
pub(crate) fn memory(bytes: u64) -> Memory {
    Memory::from(Arc::new(
        Budget::new(bytes).with_pages(everygraph_io::PAGES),
    ))
}

/// What the machine has.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Machine {
    pub(crate) cores: usize,
    pub(crate) hardware_threads: usize,
    pub(crate) memory: Option<SystemMemory>,
}

impl Machine {
    pub(crate) fn probe() -> Self {
        let hardware_threads = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
        Self {
            cores: physical_cores().unwrap_or(hardware_threads),
            hardware_threads,
            memory: ProcMemoryProbe::new().probe().ok(),
        }
    }

    /// The memory in all, if known.
    pub(crate) fn total(&self) -> Option<u64> {
        self.memory.map(|m| m.total())
    }

    /// The budget of `common`: `--memory`, or half of what is free (P-11).
    ///
    /// # Errors
    ///
    /// A `--memory` above the machine's memory in all is a usage error (P-16).
    pub(crate) fn budget(&self, common: &Common) -> Result<u64, Failure> {
        match (&common.memory, self.memory) {
            (Some((bytes, text)), Some(memory)) if *bytes > memory.total() => {
                Err(too_much(*bytes, text, &memory))
            }
            (Some((bytes, _)), _) => Ok(*bytes),
            (None, memory) => Ok(memory.map_or(UNKNOWN_FREE_BUDGET, |m| m.available() / 2)),
        }
    }

    /// The threads of `common`: `--threads`, or every hardware thread.
    pub(crate) fn threads(&self, common: &Common) -> NonZeroUsize {
        common
            .threads
            .unwrap_or(NonZeroUsize::new(self.hardware_threads).unwrap_or(NonZeroUsize::MIN))
    }
}

/// Distinct (package, core) pairs of `/proc/cpuinfo`.
fn physical_cores() -> Option<usize> {
    let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let mut cores = HashSet::new();
    let mut package = "";
    for line in info.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "physical id" => package = value.trim(),
            "core id" => {
                cores.insert((package, value.trim()));
            }
            _ => {}
        }
    }
    (!cores.is_empty()).then_some(cores.len())
}

/// Free space of the file system that holds a directory, and whether it is held in
/// memory.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Disk {
    pub(crate) free: u64,
    pub(crate) memory_backed: bool,
}

/// The disk of `dir`; `None` where it cannot be known.
pub(crate) fn disk(dir: &Path) -> Option<Disk> {
    #[cfg(unix)]
    {
        let fs = rustix::fs::statvfs(dir).ok()?;
        #[cfg(target_os = "linux")]
        let memory_backed = rustix::fs::statfs(dir)
            .is_ok_and(|s| MEMORY_FILE_SYSTEMS.contains(&u64::try_from(s.f_type).unwrap_or(0)));
        #[cfg(not(target_os = "linux"))]
        let memory_backed = false;
        Some(Disk {
            free: fs.f_bavail.saturating_mul(fs.f_frsize),
            memory_backed,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

/// A budget larger than the machine (P-16): a usage error before any plan.
fn too_much(bytes: u64, text: &str, memory: &SystemMemory) -> Failure {
    Failure::new(
        "UsageError",
        "The memory budget is more than this machine has.".into(),
        failure::USAGE,
    )
    .why(vec![format!(
        "--memory {text} asks for {}; this machine has {} of memory in all.",
        size(bytes),
        rough_size(memory.total())
    )])
    .ways(vec![
        (
            format!("--memory {}", typed(memory.available())),
            "what is free now".into(),
        ),
        (
            "leave out --memory".into(),
            format!(
                "the default, half of free RAM: {} now",
                size(memory.available() / 2)
            ),
        ),
    ])
}
