// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the machine has besides memory (`docs/ux-contract.md` P-2): the physical cores
//! the process may run on, and the storage of a directory.

use std::num::NonZeroUsize;
use std::path::Path;

use everygraph_core::Storage;
#[cfg(target_os = "linux")]
use everygraph_core::StorageKind;

/// `statfs` types (Linux `magic.h`, and the file systems' own headers) of network,
/// cluster and user-space file systems: NFS, SMB 1, SMB 2 and 3, CIFS, FUSE, Ceph, 9P,
/// Lustre, GPFS, AFS and kAFS.
#[cfg(target_os = "linux")]
const REMOTE_FILE_SYSTEMS: [u32; 11] = [
    0x6969,
    0x517b,
    0xfe53_4d42,
    0xff53_4d42,
    0x6573_5546,
    0x00c3_6400,
    0x0102_1997,
    0x0bd0_0bd0,
    0x4750_4653,
    0x5346_414f,
    0x6b41_4653,
];

/// The physical cores this process may run on: on Linux, the distinct cores (package
/// and core id, `/sys/devices/system/cpu/cpuN/topology`) of the CPUs in its affinity
/// mask, at most `std::thread::available_parallelism()`, which also follows the CPU
/// quota of its control group. Elsewhere, and where the topology cannot be read,
/// `available_parallelism()` alone, which on macOS and Windows counts hardware threads.
#[must_use]
pub fn physical_cores() -> NonZeroUsize {
    let threads = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
    #[cfg(target_os = "linux")]
    if let Some(cores) = affine_cores() {
        return cores.min(threads);
    }
    threads
}

/// The distinct cores of the CPUs in the calling thread's affinity mask.
#[cfg(target_os = "linux")]
fn affine_cores() -> Option<NonZeroUsize> {
    let set = rustix::thread::sched_getaffinity(None).ok()?;
    let mut cores = std::collections::HashSet::new();
    for cpu in (0..rustix::thread::CpuSet::MAX_CPU).filter(|&c| set.is_set(c)) {
        let topology = format!("/sys/devices/system/cpu/cpu{cpu}/topology");
        let read = |name: &str| std::fs::read_to_string(format!("{topology}/{name}")).ok();
        cores.insert((read("physical_package_id")?, read("core_id")?));
    }
    NonZeroUsize::new(cores.len())
}

/// The storage that holds `dir`: its kind and the bytes free there. `None` where it
/// cannot be read, and on platforms other than Linux.
///
/// A local device is found through `/sys/dev/block/MAJOR:MINOR`, through the slaves of
/// a device-mapper or md device (LVM, dm-crypt, RAID) to the slowest disk under it.
/// btrfs, ZFS and overlayfs report no device of their own: they are
/// [`StorageKind::Unknown`], which the planner reads as slow storage.
#[must_use]
pub fn storage(dir: &Path) -> Option<Storage> {
    #[cfg(target_os = "linux")]
    {
        let fs = rustix::fs::statvfs(dir).ok()?;
        let free_bytes = fs.f_bavail.saturating_mul(fs.f_frsize);
        let magic = crate::memory_probe::magic(dir)?;
        let kind = if crate::memory_probe::MEMORY_FILE_SYSTEMS.contains(&magic) {
            StorageKind::Memory
        } else if REMOTE_FILE_SYSTEMS.contains(&magic) {
            StorageKind::Remote
        } else {
            rustix::fs::stat(dir).map_or(StorageKind::Unknown, |s| {
                let dev = s.st_dev;
                let link = format!(
                    "/sys/dev/block/{}:{}",
                    rustix::fs::major(dev),
                    rustix::fs::minor(dev)
                );
                std::fs::canonicalize(link).map_or(StorageKind::Unknown, |d| block_kind(&d, 0))
            })
        };
        Some(Storage { kind, free_bytes })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dir;
        None
    }
}

/// The kind of the block device at `device` (a directory of `/sys/devices`): the
/// slowest of its slaves, if it has any; `NVMe` by its name; otherwise its
/// `queue/rotational`, or that of the disk the partition is on.
#[cfg(target_os = "linux")]
fn block_kind(device: &Path, depth: u32) -> StorageKind {
    let rank = |k: &StorageKind| match k {
        StorageKind::Hdd => 0,
        StorageKind::Unknown => 1,
        StorageKind::Ssd => 2,
        _ => 3,
    };
    let slaves: Vec<StorageKind> = std::fs::read_dir(device.join("slaves"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::canonicalize(e.path()).ok())
        .map(|s| {
            if depth < 8 {
                block_kind(&s, depth + 1)
            } else {
                StorageKind::Unknown
            }
        })
        .collect();
    if let Some(slowest) = slaves.into_iter().min_by_key(rank) {
        return slowest;
    }
    let name = device
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let rotational = |d: &Path| {
        std::fs::read_to_string(d.join("queue/rotational"))
            .ok()
            .map(|s| s.trim() == "1")
    };
    match rotational(device).or_else(|| device.parent().and_then(rotational)) {
        _ if name.starts_with("nvme") => StorageKind::Nvme,
        Some(true) => StorageKind::Hdd,
        Some(false) => StorageKind::Ssd,
        None => StorageKind::Unknown,
    }
}
