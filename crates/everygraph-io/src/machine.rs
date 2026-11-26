// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the machine has besides memory (`docs/ux-contract.md` P-2): its physical cores,
//! and the storage of a directory.

use std::collections::HashSet;
use std::path::Path;

use everygraph_core::Storage;
#[cfg(target_os = "linux")]
use everygraph_core::StorageKind;

/// `statfs` types (Linux `magic.h`) of network and user-space file systems: NFS, SMB 1,
/// SMB 2 and 3, CIFS, FUSE, Ceph, 9P.
#[cfg(target_os = "linux")]
const REMOTE_FILE_SYSTEMS: [u64; 7] = [
    0x6969,
    0x517b,
    0xfe53_4d42,
    0xff53_4d42,
    0x6573_5546,
    0x00c3_6400,
    0x0102_1997,
];

/// The physical cores of the machine: distinct (package, core) pairs of
/// `/proc/cpuinfo`. `None` where that cannot be read.
#[must_use]
pub fn physical_cores() -> Option<usize> {
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

/// The storage that holds `dir`: its kind and the bytes free there. `None` where it
/// cannot be read; the kind is [`StorageKind::Unknown`] on platforms other than Linux.
#[must_use]
pub fn storage(dir: &Path) -> Option<Storage> {
    #[cfg(target_os = "linux")]
    {
        let fs = rustix::fs::statvfs(dir).ok()?;
        let free_bytes = fs.f_bavail.saturating_mul(fs.f_frsize);
        let kind =
            u64::try_from(rustix::fs::statfs(dir).ok()?.f_type).map_or(StorageKind::Unknown, |t| {
                if crate::memory_probe::MEMORY_FILE_SYSTEMS.contains(&t) {
                    StorageKind::Memory
                } else if REMOTE_FILE_SYSTEMS.contains(&t) {
                    StorageKind::Remote
                } else {
                    block_device(dir)
                }
            });
        Some(Storage { kind, free_bytes })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dir;
        None
    }
}

/// The kind of the block device of `dir`, from `/sys/dev/block/MAJOR:MINOR`: `NVMe` by its
/// name, then `queue/rotational` of the device or of the disk the partition is on.
#[cfg(target_os = "linux")]
fn block_device(dir: &Path) -> StorageKind {
    let Ok(stat) = rustix::fs::stat(dir) else {
        return StorageKind::Unknown;
    };
    let dev = stat.st_dev;
    let link = format!(
        "/sys/dev/block/{}:{}",
        rustix::fs::major(dev),
        rustix::fs::minor(dev)
    );
    let Ok(device) = std::fs::canonicalize(&link) else {
        return StorageKind::Unknown;
    };
    let name = device
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let rotational = |d: &Path| {
        std::fs::read_to_string(d.join("queue/rotational"))
            .ok()
            .map(|s| s.trim() == "1")
    };
    let spins = rotational(&device).or_else(|| device.parent().and_then(rotational));
    match spins {
        _ if name.starts_with("nvme") => StorageKind::Nvme,
        Some(true) => StorageKind::Hdd,
        Some(false) => StorageKind::Ssd,
        None => StorageKind::Unknown,
    }
}
