// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the harness reads from `/proc` and `/sys`: load average, `MemAvailable`, CPU
//! time, the I/O counters of a process, its cgroup, and the files of a cgroup v2.
//!
//! Every file has a pure parser (tested on fixture text); [`Host`] reads them under a
//! root directory, `/` in production and a fixture tree in tests.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The ticks per second of `/proc/stat` (`USER_HZ`). Linux fixes it at 100 for user space
/// on every architecture it supports, whatever the kernel's own `HZ`.
pub const USER_HZ: f64 = 100.0;

/// The three load averages of `/proc/loadavg`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LoadAvg {
    /// 1-minute average.
    pub one: f64,
    /// 5-minute average.
    pub five: f64,
    /// 15-minute average.
    pub fifteen: f64,
}

/// Parses `/proc/loadavg` (`0.61 0.20 0.06 1/1234 5678`).
///
/// # Errors
///
/// When the first three fields are not numbers.
pub fn parse_loadavg(text: &str) -> Result<LoadAvg, String> {
    let mut f = text.split_whitespace().map(str::parse::<f64>);
    match (f.next(), f.next(), f.next()) {
        (Some(Ok(one)), Some(Ok(five)), Some(Ok(fifteen))) => Ok(LoadAvg { one, five, fifteen }),
        _ => Err(format!("unexpected /proc/loadavg: {text:?}")),
    }
}

/// Parses `/proc/meminfo` into bytes per key (`MemAvailable` → bytes).
#[must_use]
pub fn parse_meminfo(text: &str) -> BTreeMap<String, u64> {
    text.lines()
        .filter_map(|l| {
            let (key, rest) = l.split_once(':')?;
            let mut parts = rest.split_whitespace();
            let n: u64 = parts.next()?.parse().ok()?;
            let bytes = match parts.next() {
                Some("kB") => n.checked_mul(1024)?,
                None => n,
                Some(_) => return None,
            };
            Some((key.trim().to_string(), bytes))
        })
        .collect()
}

/// CPU time of the whole machine, from the `cpu` line of `/proc/stat`, in ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuTimes {
    /// Time spent running something: user, nice, system, irq, softirq and steal.
    pub busy: u64,
    /// Busy plus idle and iowait.
    pub total: u64,
}

/// Parses the aggregate `cpu` line of `/proc/stat`.
///
/// # Errors
///
/// When there is no such line or it has fewer than eight numbers.
pub fn parse_proc_stat(text: &str) -> Result<CpuTimes, String> {
    let line = text
        .lines()
        .find(|l| l.starts_with("cpu "))
        .ok_or("no aggregate cpu line in /proc/stat")?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map_while(|x| x.parse().ok())
        .collect();
    let [user, nice, system, idle, iowait, irq, softirq, steal, ..] = v[..] else {
        return Err(format!("unexpected /proc/stat cpu line: {line:?}"));
    };
    // guest and guest_nice are already counted in user and nice.
    let busy = user + nice + system + irq + softirq + steal;
    Ok(CpuTimes {
        busy,
        total: busy + idle + iowait,
    })
}

/// Storage traffic of a process and of the children it has waited for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IoCounters {
    /// Bytes fetched from storage (page-cache hits not counted; `O_DIRECT` counted).
    pub read_bytes: u64,
    /// Bytes sent to storage, or dirtied in the page cache for it.
    pub write_bytes: u64,
}

/// Parses `/proc/<pid>/io`.
///
/// # Errors
///
/// When `read_bytes` or `write_bytes` is missing.
pub fn parse_proc_io(text: &str) -> Result<IoCounters, String> {
    let kv = parse_flat_keyed(&text.replace(':', " "));
    match (kv.get("read_bytes"), kv.get("write_bytes")) {
        (Some(&read_bytes), Some(&write_bytes)) => Ok(IoCounters {
            read_bytes,
            write_bytes,
        }),
        _ => Err(format!("unexpected /proc/self/io: {text:?}")),
    }
}

/// The cgroup v2 path of a process, from `/proc/<pid>/cgroup` (the `0::` line).
#[must_use]
pub fn parse_proc_cgroup(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(|p| p.trim().to_string())
}

/// Parses the flat keyed files of cgroup v2 (`memory.events`, `memory.stat`,
/// `cpu.stat`): one `key value` pair per line. Lines that are not are skipped.
#[must_use]
pub fn parse_flat_keyed(text: &str) -> BTreeMap<String, u64> {
    text.lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            let k = p.next()?;
            let v = p.next()?.parse().ok()?;
            Some((k.to_string(), v))
        })
        .collect()
}

/// Parses a single-value cgroup file (`memory.max`, `memory.peak`): `None` for `max`.
///
/// # Errors
///
/// When the content is neither a number nor `max`.
pub fn parse_limit(text: &str) -> Result<Option<u64>, String> {
    match text.trim() {
        "max" => Ok(None),
        t => t
            .parse()
            .map(Some)
            .map_err(|_| format!("unexpected cgroup value {t:?}")),
    }
}

/// What `/proc/cpuinfo` says about the processor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CpuInfo {
    /// `model name` of the first processor.
    pub model: String,
    /// Logical processors.
    pub logical: u32,
    /// Distinct (physical id, core id) pairs; 0 when the file does not say.
    pub physical_cores: u32,
}

/// Parses `/proc/cpuinfo`.
#[must_use]
pub fn parse_cpuinfo(text: &str) -> CpuInfo {
    let mut info = CpuInfo::default();
    let mut cores = std::collections::BTreeSet::new();
    let (mut package, mut core) = (None, None);
    for block in text.split("\n\n").filter(|b| b.contains("processor")) {
        info.logical += 1;
        for line in block.lines() {
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            match k.trim() {
                "model name" if info.model.is_empty() => info.model = v.trim().to_string(),
                "physical id" => package = Some(v.trim().to_string()),
                "core id" => core = Some(v.trim().to_string()),
                _ => {}
            }
        }
        if let (Some(p), Some(c)) = (package.take(), core.take()) {
            cores.insert((p, c));
        }
    }
    info.physical_cores = u32::try_from(cores.len()).unwrap_or(u32::MAX);
    info
}

/// The selected value of a `/sys` choice file (`always [madvise] never` → `madvise`).
#[must_use]
pub fn parse_selected(text: &str) -> String {
    text.split_whitespace()
        .find_map(|w| w.strip_prefix('[').and_then(|w| w.strip_suffix(']')))
        .unwrap_or_else(|| text.trim())
        .to_string()
}

/// Reads the files above under a root directory.
#[derive(Debug, Clone)]
pub struct Host {
    root: PathBuf,
}

impl Default for Host {
    fn default() -> Self {
        Self::new("/")
    }
}

impl Host {
    /// Files are read under `root` (`/` for the real machine).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `root/relative`.
    #[must_use]
    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative.trim_start_matches('/'))
    }

    /// Reads a file under the root.
    ///
    /// # Errors
    ///
    /// The file cannot be read; the message names it.
    pub fn read(&self, relative: &str) -> Result<String, String> {
        let p = self.path(relative);
        std::fs::read_to_string(&p).map_err(|e| format!("reading {}: {e}", p.display()))
    }

    /// The load averages.
    ///
    /// # Errors
    ///
    /// The file is missing or malformed.
    pub fn loadavg(&self) -> Result<LoadAvg, String> {
        parse_loadavg(&self.read("proc/loadavg")?)
    }

    /// `MemAvailable` in bytes.
    ///
    /// # Errors
    ///
    /// The file is missing or has no `MemAvailable`.
    pub fn mem_available(&self) -> Result<u64, String> {
        self.meminfo_key("MemAvailable")
    }

    /// One `/proc/meminfo` value in bytes.
    ///
    /// # Errors
    ///
    /// The file is missing or has no such key.
    pub fn meminfo_key(&self, key: &str) -> Result<u64, String> {
        parse_meminfo(&self.read("proc/meminfo")?)
            .get(key)
            .copied()
            .ok_or_else(|| format!("/proc/meminfo has no {key}"))
    }

    /// Machine CPU time.
    ///
    /// # Errors
    ///
    /// The file is missing or malformed.
    pub fn cpu_times(&self) -> Result<CpuTimes, String> {
        parse_proc_stat(&self.read("proc/stat")?)
    }

    /// I/O counters of this process (including waited-for children).
    ///
    /// # Errors
    ///
    /// The file is missing or malformed.
    pub fn self_io(&self) -> Result<IoCounters, String> {
        parse_proc_io(&self.read("proc/self/io")?)
    }

    /// The directory of this process's cgroup under `sys/fs/cgroup`.
    ///
    /// # Errors
    ///
    /// No cgroup v2 entry (cgroup v1 hosts are not supported).
    pub fn self_cgroup(&self) -> Result<PathBuf, String> {
        let p = parse_proc_cgroup(&self.read("proc/self/cgroup")?).ok_or(
            "this process has no cgroup v2 entry; the harness needs the unified hierarchy",
        )?;
        Ok(self.path("sys/fs/cgroup").join(p.trim_start_matches('/')))
    }
}

/// A cgroup v2 directory.
#[derive(Debug, Clone)]
pub struct Cgroup {
    dir: PathBuf,
}

impl Cgroup {
    /// The cgroup at `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Its directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn read(&self, name: &str) -> Result<String, String> {
        let p = self.dir.join(name);
        std::fs::read_to_string(&p).map_err(|e| format!("reading {}: {e}", p.display()))
    }

    /// `memory.max` (`None`: no limit).
    ///
    /// # Errors
    ///
    /// Unreadable or malformed.
    pub fn memory_max(&self) -> Result<Option<u64>, String> {
        parse_limit(&self.read("memory.max")?)
    }

    /// `memory.swap.max` (`None`: no limit).
    ///
    /// # Errors
    ///
    /// Unreadable or malformed.
    pub fn swap_max(&self) -> Result<Option<u64>, String> {
        parse_limit(&self.read("memory.swap.max")?)
    }

    /// `memory.peak`: the largest memory use of the cgroup since it was created,
    /// anonymous, file-backed and kernel memory together.
    ///
    /// # Errors
    ///
    /// Unreadable (kernels before 5.19 do not have it) or malformed.
    pub fn memory_peak(&self) -> Result<u64, String> {
        parse_limit(&self.read("memory.peak")?)?.ok_or_else(|| "memory.peak says max".into())
    }

    /// `memory.events`: `low`, `high`, `max`, `oom`, `oom_kill`, `oom_group_kill`.
    ///
    /// # Errors
    ///
    /// Unreadable.
    pub fn memory_events(&self) -> Result<BTreeMap<String, u64>, String> {
        Ok(parse_flat_keyed(&self.read("memory.events")?))
    }

    /// `memory.stat` (bytes, and event counts).
    ///
    /// # Errors
    ///
    /// Unreadable.
    pub fn memory_stat(&self) -> Result<BTreeMap<String, u64>, String> {
        Ok(parse_flat_keyed(&self.read("memory.stat")?))
    }

    /// `cpu.stat` (`usage_usec`, `user_usec`, `system_usec`, …).
    ///
    /// # Errors
    ///
    /// Unreadable.
    pub fn cpu_stat(&self) -> Result<BTreeMap<String, u64>, String> {
        Ok(parse_flat_keyed(&self.read("cpu.stat")?))
    }

    /// The process ids in the cgroup.
    ///
    /// # Errors
    ///
    /// Unreadable.
    pub fn procs(&self) -> Result<Vec<u32>, String> {
        Ok(self
            .read("cgroup.procs")?
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loadavg_and_meminfo() {
        let l = parse_loadavg("0.61 0.20 0.06 1/1234 5678\n").expect("valid");
        assert_eq!((l.one, l.five, l.fifteen), (0.61, 0.20, 0.06));
        assert!(parse_loadavg("").is_err());
        let m = parse_meminfo(
            "MemTotal:       64000000 kB\nMemAvailable:   62000000 kB\nHugePages_Total:       0\n",
        );
        assert_eq!(m["MemAvailable"], 62_000_000 * 1024);
        assert_eq!(m["HugePages_Total"], 0);
    }

    #[test]
    fn proc_stat_busy_excludes_idle_and_iowait() {
        let text = "cpu  100 5 20 1000 7 3 2 1 50 0\ncpu0 1 2 3 4 5 6 7 8 9 10\nintr 1\n";
        let t = parse_proc_stat(text).expect("valid");
        assert_eq!(t.busy, 100 + 5 + 20 + 3 + 2 + 1);
        assert_eq!(t.total, t.busy + 1000 + 7);
        assert!(parse_proc_stat("cpu0 1 2\n").is_err());
    }

    #[test]
    fn proc_io_and_cgroup() {
        let io = "rchar: 104876132\nwchar: 104857600\nsyscr: 132\nsyscw: 100\nread_bytes: 52506624\nwrite_bytes: 52428800\ncancelled_write_bytes: 0\n";
        assert_eq!(
            parse_proc_io(io),
            Ok(IoCounters {
                read_bytes: 52_506_624,
                write_bytes: 52_428_800
            })
        );
        assert_eq!(
            parse_proc_cgroup("0::/user.slice/user-1000.slice/app.slice/run-p1.scope\n").as_deref(),
            Some("/user.slice/user-1000.slice/app.slice/run-p1.scope")
        );
        assert_eq!(parse_proc_cgroup("1:name=systemd:/x\n"), None);
    }

    /// Fixtures copied from a `systemd-run --user --scope -p MemoryMax=1G` scope on the
    /// lab machine (kernel 6.12).
    #[test]
    fn cgroup_files() {
        let events = "low 0\nhigh 0\nmax 12\noom 1\noom_kill 1\noom_group_kill 0\n";
        let e = parse_flat_keyed(events);
        assert_eq!((e["max"], e["oom_kill"]), (12, 1));
        let stat = "anon 237568\nfile 4096\nkernel 303104\nkernel_stack 16384\npagetables 102400\nsec_pagetables 0\npercpu 0\n";
        let s = parse_flat_keyed(stat);
        assert_eq!((s["anon"], s["file"]), (237_568, 4096));
        let cpu = "usage_usec 5638\nuser_usec 0\nsystem_usec 5638\nnice_usec 0\nnr_periods 0\n";
        assert_eq!(parse_flat_keyed(cpu)["usage_usec"], 5638);
        assert_eq!(parse_limit("1073741824\n"), Ok(Some(1 << 30)));
        assert_eq!(parse_limit("max\n"), Ok(None));
        assert!(parse_limit("lots").is_err());
    }

    #[test]
    fn cpuinfo_counts_cores_and_threads() {
        let block = |p: u32, core: u32| {
            format!(
                "processor\t: {p}\nmodel name\t: AMD Ryzen 9 7950X 16-Core Processor\nphysical id\t: 0\ncore id\t\t: {core}\n\n"
            )
        };
        let text: String = (0..32).map(|p| block(p, p % 16)).collect();
        let info = parse_cpuinfo(&text);
        assert_eq!(info.model, "AMD Ryzen 9 7950X 16-Core Processor");
        assert_eq!((info.logical, info.physical_cores), (32, 16));
        assert_eq!(parse_selected("always [madvise] never\n"), "madvise");
        assert_eq!(parse_selected("performance\n"), "performance");
    }
}
