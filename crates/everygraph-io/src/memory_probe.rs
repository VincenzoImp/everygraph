// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`ProcMemoryProbe`]: what a Linux machine, and the control group the process runs
//! in, can give.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use everygraph_core::{MemoryProbe, SystemMemory};

/// Cgroup v1 reports "no limit" as a number near `i64::MAX` rounded to a page; any
/// value from here up means unlimited.
const V1_UNLIMITED: u64 = 1 << 62;

/// Reads `/proc/meminfo` and the memory limits of the process's control group.
///
/// The limit is the tightest one on the way from the process's own group to the root
/// of the hierarchy: `systemd-run -p MemoryMax=…` sets it on the scope, a container
/// runtime or a parent slice may set it higher up, and a group whose own
/// `memory.max` is `max` is still bounded by its parents. Both cgroup v2 and the
/// memory controller of v1 are read. What is free under a limit is the limit minus the
/// group's usage, not counting the page cache on its file LRU lists (reclaimable, as
/// `MemAvailable` treats it on the machine; shmem and tmpfs are not), at the level
/// that leaves the least.
///
/// A level whose files are missing is skipped; a value that cannot be parsed is an
/// error, never a guess.
#[derive(Clone, Debug)]
pub struct ProcMemoryProbe {
    proc: PathBuf,
    cgroup: PathBuf,
}

impl ProcMemoryProbe {
    /// The probe of this machine: `/proc` and `/sys/fs/cgroup`.
    #[must_use]
    pub fn new() -> Self {
        Self::with_roots("/proc", "/sys/fs/cgroup")
    }

    /// A probe that reads `proc` instead of `/proc` and `cgroup` instead of
    /// `/sys/fs/cgroup` (for tests, and for a probe run on another root).
    pub fn with_roots(proc: impl Into<PathBuf>, cgroup: impl Into<PathBuf>) -> Self {
        Self {
            proc: proc.into(),
            cgroup: cgroup.into(),
        }
    }
}

impl Default for ProcMemoryProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryProbe for ProcMemoryProbe {
    fn probe(&self) -> io::Result<SystemMemory> {
        let meminfo = read(&self.proc.join("meminfo"))?;
        let total = meminfo_field(&meminfo, "MemTotal")?;
        let available = meminfo_field(&meminfo, "MemAvailable")?;
        let (limit, limit_available) = match read_optional(&self.proc.join("self/cgroup"))? {
            Some(membership) => self.limits(&membership)?,
            None => (None, None),
        };
        Ok(SystemMemory::new(total, available, limit, limit_available))
    }
}

impl ProcMemoryProbe {
    /// The tightest limit and the least free memory under a limit, from the lines of
    /// `/proc/self/cgroup`.
    fn limits(&self, membership: &str) -> io::Result<(Option<u64>, Option<u64>)> {
        let mut limit: Option<u64> = None;
        let mut free: Option<u64> = None;
        for line in membership.lines() {
            let mut fields = line.splitn(3, ':');
            let (Some(_), Some(controllers), Some(path)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let (root, files) = if controllers.is_empty() {
                (self.cgroup.clone(), &V2)
            } else if controllers.split(',').any(|c| c == "memory") {
                (self.cgroup.join("memory"), &V1)
            } else {
                continue;
            };
            let leaf = root.join(path.trim_start_matches('/'));
            for dir in leaf.ancestors().take_while(|dir| dir.starts_with(&root)) {
                let Some(max) = files.limit(dir)? else {
                    continue;
                };
                limit = Some(limit.map_or(max, |l| l.min(max)));
                let used = files.unreclaimable_usage(dir)?.unwrap_or(0);
                let left = max.saturating_sub(used);
                free = Some(free.map_or(left, |f| f.min(left)));
            }
        }
        Ok((limit, free))
    }
}

/// The file names of one cgroup version.
struct Files {
    limit: &'static str,
    usage: &'static str,
    /// The `memory.stat` keys of the pages on the file LRU lists: the page cache that
    /// reclaim can drop. Not `file` (v2) or `total_cache` (v1): those include shmem
    /// (tmpfs, `/dev/shm`), which lives on the anonymous LRU lists and, without swap,
    /// cannot be reclaimed at all. The LRU keys also leave out unevictable (mlocked)
    /// file pages, which `file − shmem` would count as free.
    reclaimable: [&'static str; 2],
}

const V2: Files = Files {
    limit: "memory.max",
    usage: "memory.current",
    reclaimable: ["active_file", "inactive_file"],
};

const V1: Files = Files {
    limit: "memory.limit_in_bytes",
    usage: "memory.usage_in_bytes",
    reclaimable: ["total_active_file", "total_inactive_file"],
};

impl Files {
    /// The limit set on `dir`, if any.
    fn limit(&self, dir: &Path) -> io::Result<Option<u64>> {
        let Some(text) = read_optional(&dir.join(self.limit))? else {
            return Ok(None);
        };
        let text = text.trim();
        if text == "max" {
            return Ok(None);
        }
        let bytes = parse(text, &dir.join(self.limit))?;
        Ok((bytes < V1_UNLIMITED).then_some(bytes))
    }

    /// What the group uses, minus the page cache reclaim can drop. A missing key counts
    /// as nothing reclaimable.
    fn unreclaimable_usage(&self, dir: &Path) -> io::Result<Option<u64>> {
        let Some(usage) = read_optional(&dir.join(self.usage))? else {
            return Ok(None);
        };
        let usage = parse(usage.trim(), &dir.join(self.usage))?;
        let mut reclaimable: u64 = 0;
        if let Some(stat) = read_optional(&dir.join("memory.stat"))? {
            for line in stat.lines() {
                let Some((key, value)) = line.split_once(' ') else {
                    continue;
                };
                if self.reclaimable.contains(&key) {
                    let bytes = parse(value.trim(), &dir.join("memory.stat"))?;
                    reclaimable = reclaimable.saturating_add(bytes);
                }
            }
        }
        Ok(Some(usage.saturating_sub(reclaimable)))
    }
}

fn read(path: &Path) -> io::Result<String> {
    fs::read_to_string(path)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}

/// The file's text, or `None` if it does not exist (a controller that is not enabled,
/// a level without limits).
fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io::Error::new(e.kind(), format!("{}: {e}", path.display()))),
    }
}

fn parse(text: &str, path: &Path) -> io::Result<u64> {
    text.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: not a number of bytes: {text:?}", path.display()),
        )
    })
}

/// A field of `/proc/meminfo`, in bytes (the file gives kB).
fn meminfo_field(meminfo: &str, name: &str) -> io::Result<u64> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("/proc/meminfo: no readable {name} line"),
        )
    };
    let line = meminfo
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
        .ok_or_else(invalid)?;
    let kib: u64 = line
        .trim()
        .strip_suffix("kB")
        .ok_or_else(invalid)?
        .trim()
        .parse()
        .map_err(|_| invalid())?;
    kib.checked_mul(1024).ok_or_else(invalid)
}

/// `statfs` types of the file systems held in memory (Linux `magic.h`: tmpfs, ramfs).
#[cfg(target_os = "linux")]
pub(crate) const MEMORY_FILE_SYSTEMS: [u32; 2] = [0x0102_1994, 0x8584_58f6];

/// The `statfs` type of the file system that holds `dir`: a 32-bit magic number, which
/// i686 stores signed.
#[cfg(target_os = "linux")]
pub(crate) fn magic(dir: &Path) -> Option<u32> {
    // `allow`, not `expect`: the field is an `i64` on 64-bit targets and an `i32` on
    // i686, so which of the two lints fires depends on the target.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "statfs magic numbers are 32 bits"
    )]
    rustix::fs::statfs(dir).ok().map(|s| s.f_type as u32)
}

/// Whether the file system that holds `dir` keeps its files in memory (tmpfs, ramfs):
/// scratch there is memory (`docs/ux-contract.md` P-18). `false` where this cannot be
/// told (other platforms, or a directory that cannot be queried).
#[must_use]
pub fn memory_backed(dir: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        magic(dir).is_some_and(|t| MEMORY_FILE_SYSTEMS.contains(&t))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dir;
        false
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const GIB: u64 = 1 << 30;

    /// A fake `/proc` and `/sys/fs/cgroup` under a fresh directory, removed on drop.
    struct Root(PathBuf);

    impl Root {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "everygraph-probe-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            let root = Self(dir);
            root.write(
                "proc/meminfo",
                "MemTotal:       65536000 kB\nMemFree:  1 kB\nMemAvailable:   40960000 kB\n",
            );
            root
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn probe(&self) -> io::Result<SystemMemory> {
            ProcMemoryProbe::with_roots(self.0.join("proc"), self.0.join("cgroup")).probe()
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn without_a_limit_the_machine_is_the_bound() {
        let root = Root::new();
        root.write("proc/self/cgroup", "0::/user.slice/a.scope\n");
        root.write("cgroup/user.slice/a.scope/memory.max", "max\n");
        root.write("cgroup/user.slice/memory.max", "max\n");
        let memory = root.probe().unwrap();
        assert_eq!(
            memory,
            SystemMemory::new(65_536_000 * 1024, 40_960_000 * 1024, None, None)
        );
    }

    /// The limit sits on a parent and the leaf says `max`: the lab's accountant read
    /// only the leaf, found nothing, and panicked.
    #[test]
    fn a_limit_on_a_parent_group_is_found() {
        let root = Root::new();
        root.write("proc/self/cgroup", "0::/slice/job.scope\n");
        root.write("cgroup/slice/job.scope/memory.max", "max\n");
        root.write("cgroup/slice/job.scope/memory.current", "1073741824\n");
        root.write("cgroup/slice/memory.max", &format!("{}\n", 8 * GIB));
        root.write("cgroup/slice/memory.current", &format!("{}\n", 3 * GIB));
        root.write(
            "cgroup/slice/memory.stat",
            &format!(
                "anon {}\nfile {GIB}\nkernel 0\nshmem 0\nactive_file {}\ninactive_file {}\n",
                2 * GIB,
                GIB / 4,
                3 * GIB / 4
            ),
        );
        let memory = root.probe().unwrap();
        assert_eq!(memory.limit, Some(8 * GIB));
        // 8 GiB minus 3 GiB used, of which 1 GiB is file cache.
        assert_eq!(memory.limit_available, Some(6 * GIB));
        assert_eq!((memory.total(), memory.available()), (8 * GIB, 6 * GIB));
    }

    /// tmpfs and `/dev/shm` pages are counted in `file` (v2) and `total_cache` (v1) but
    /// sit on the anonymous LRU lists: without swap they free nothing, so 400 MiB in
    /// `/dev/shm` must not count as free.
    #[test]
    fn shared_memory_is_not_reclaimable() {
        let root = Root::new();
        root.write("proc/self/cgroup", "0::/job\n");
        root.write("cgroup/job/memory.max", &format!("{}\n", 4 * GIB));
        root.write("cgroup/job/memory.current", &format!("{}\n", 2 * GIB));
        root.write(
            "cgroup/job/memory.stat",
            &format!("anon {GIB}\nfile {GIB}\nshmem {GIB}\nactive_file 0\ninactive_file 0\n"),
        );
        assert_eq!(root.probe().unwrap().limit_available, Some(2 * GIB));

        let root = Root::new();
        root.write("proc/self/cgroup", "4:memory:/job\n");
        root.write(
            "cgroup/memory/job/memory.limit_in_bytes",
            &format!("{}\n", 4 * GIB),
        );
        root.write(
            "cgroup/memory/job/memory.usage_in_bytes",
            &format!("{}\n", 2 * GIB),
        );
        root.write(
            "cgroup/memory/job/memory.stat",
            &format!(
                "total_cache {GIB}\ntotal_shmem {GIB}\ntotal_active_file 0\ntotal_inactive_file 0\n"
            ),
        );
        assert_eq!(root.probe().unwrap().limit_available, Some(2 * GIB));
    }

    #[test]
    fn the_tightest_level_wins() {
        let root = Root::new();
        root.write("proc/self/cgroup", "0::/outer/inner\n");
        root.write("cgroup/outer/inner/memory.max", &format!("{}\n", 4 * GIB));
        root.write("cgroup/outer/inner/memory.current", &format!("{GIB}\n"));
        root.write("cgroup/outer/memory.max", &format!("{}\n", 6 * GIB));
        root.write("cgroup/outer/memory.current", &format!("{}\n", 5 * GIB));
        let memory = root.probe().unwrap();
        assert_eq!(memory.limit, Some(4 * GIB));
        assert_eq!(
            memory.limit_available,
            Some(GIB),
            "the outer group has 1 GiB left"
        );
    }

    #[test]
    fn cgroup_v1_is_read_and_its_unlimited_value_ignored() {
        let root = Root::new();
        root.write(
            "proc/self/cgroup",
            "12:cpu,cpuacct:/x\n7:memory:/docker/abc\n0::/\n",
        );
        root.write(
            "cgroup/memory/docker/abc/memory.limit_in_bytes",
            &format!("{}\n", 2 * GIB),
        );
        root.write(
            "cgroup/memory/docker/abc/memory.usage_in_bytes",
            &format!("{GIB}\n"),
        );
        root.write(
            "cgroup/memory/memory.limit_in_bytes",
            "9223372036854771712\n",
        );
        let memory = root.probe().unwrap();
        assert_eq!(memory.limit, Some(2 * GIB));
        assert_eq!(memory.limit_available, Some(GIB));
    }

    #[test]
    fn missing_cgroup_files_mean_no_limit() {
        let root = Root::new();
        let memory = root.probe().unwrap();
        assert_eq!(memory.limit, None);
        root.write("proc/self/cgroup", "0::/nowhere\n");
        assert_eq!(root.probe().unwrap().limit, None);
    }

    #[test]
    fn unreadable_values_are_errors_not_guesses() {
        let root = Root::new();
        root.write("proc/self/cgroup", "0::/a\n");
        root.write("cgroup/a/memory.max", "lots\n");
        let error = root.probe().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("memory.max"), "{error}");

        let root = Root::new();
        root.write("proc/meminfo", "MemTotal: 100 kB\n");
        let error = root.probe().unwrap_err();
        assert!(error.to_string().contains("MemAvailable"), "{error}");

        let root = Root::new();
        let _ = fs::remove_file(root.0.join("proc/meminfo"));
        assert_eq!(root.probe().unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_machine_can_be_probed() {
        let memory = ProcMemoryProbe::new().probe().unwrap();
        assert!(memory.machine_total > 0);
        assert!(memory.available() <= memory.total());
    }
}
