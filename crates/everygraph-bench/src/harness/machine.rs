// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The machine fingerprint and the git revision recorded in every row.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use super::procfs::{Host, parse_cpuinfo, parse_selected};

/// What the machine was when a row was measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    /// Stable across reboots and kernel updates: a hash of CPU model, core and thread
    /// counts and memory size in GiB. Rows with equal ids come from the same hardware.
    pub id: String,
    /// `model name` from `/proc/cpuinfo`.
    pub cpu_model: String,
    /// Physical cores.
    pub cores: u32,
    /// Logical processors.
    pub threads: u32,
    /// `/sys/devices/system/cpu/smt/control` (`on`, `off`, `notsupported`, …).
    pub smt: Option<String>,
    /// `MemTotal` in bytes.
    pub memory_bytes: u64,
    /// `/proc/sys/kernel/osrelease`.
    pub kernel: String,
    /// The distinct `scaling_governor` values of all CPUs (`performance`, `powersave`).
    pub governors: Vec<String>,
    /// `/sys/devices/system/cpu/cpufreq/boost`, when the driver exposes it.
    pub boost: Option<bool>,
    /// Transparent huge pages mode (`always`, `madvise`, `never`).
    pub thp: Option<String>,
}

impl Fingerprint {
    /// Reads the fingerprint of the machine whose `/proc` and `/sys` are under `host`.
    ///
    /// # Errors
    ///
    /// `/proc/cpuinfo`, `/proc/meminfo` or the kernel release cannot be read. The other
    /// fields are recorded as absent when their files are missing.
    pub fn read(host: &Host) -> Result<Self, String> {
        let cpu = parse_cpuinfo(&host.read("proc/cpuinfo")?);
        let memory_bytes = host.meminfo_key("MemTotal")?;
        let kernel = host.read("proc/sys/kernel/osrelease")?.trim().to_string();
        let opt = |p: &str| host.read(p).ok().map(|t| parse_selected(&t));
        let mut governors: Vec<String> = std::fs::read_dir(host.path("sys/devices/system/cpu"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let n = e.file_name();
                let n = n.to_string_lossy();
                n.strip_prefix("cpu")
                    .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
            })
            .filter_map(|e| std::fs::read_to_string(e.path().join("cpufreq/scaling_governor")).ok())
            .map(|g| g.trim().to_string())
            .collect();
        governors.sort();
        governors.dedup();
        let cores = if cpu.physical_cores > 0 {
            cpu.physical_cores
        } else {
            cpu.logical
        };
        let id_source = format!(
            "{}|{}|{}|{}",
            cpu.model,
            cores,
            cpu.logical,
            memory_bytes >> 30
        );
        let id = crate::datasets::hex(&<sha2::Sha256 as sha2::Digest>::digest(
            id_source.as_bytes(),
        ));
        Ok(Self {
            id: id[..16].to_string(),
            cpu_model: cpu.model,
            cores,
            threads: cpu.logical,
            smt: opt("sys/devices/system/cpu/smt/control"),
            memory_bytes,
            kernel,
            governors,
            boost: opt("sys/devices/system/cpu/cpufreq/boost").map(|b| b == "1"),
            thp: opt("sys/kernel/mm/transparent_hugepage/enabled"),
        })
    }

    /// One line for reports: `AMD Ryzen 9 7950X …, 16 cores / 32 threads, 61.9 GiB, …`.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{}, {} cores / {} threads, {}, Linux {}",
            self.cpu_model,
            self.cores,
            self.threads,
            super::units::human_size(self.memory_bytes),
            self.kernel
        );
        if !self.governors.is_empty() {
            s.push_str(&format!(", governor {}", self.governors.join("/")));
        }
        if let Some(smt) = &self.smt {
            s.push_str(&format!(", SMT {smt}"));
        }
        if let Some(b) = self.boost {
            s.push_str(if b { ", boost on" } else { ", boost off" });
        }
        if let Some(t) = &self.thp {
            s.push_str(&format!(", THP {t}"));
        }
        s
    }
}

/// The revision of the repository a run was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitInfo {
    /// Full commit id of `HEAD`.
    pub revision: String,
    /// Tracked files differ from `HEAD` (untracked files do not count: new result files
    /// are untracked until committed).
    pub dirty: bool,
}

impl GitInfo {
    /// Asks `git` about the repository at `repo`.
    ///
    /// # Errors
    ///
    /// `git` cannot be run or `repo` is not a repository.
    pub fn read(repo: &Path) -> Result<Self, String> {
        let git = |args: &[&str]| -> Result<String, String> {
            let out = Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .output()
                .map_err(|e| format!("running git: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "git {} in {}: {}",
                    args.join(" "),
                    repo.display(),
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        };
        Ok(Self {
            revision: git(&["rev-parse", "HEAD"])?.trim().to_string(),
            dirty: !git(&["status", "--porcelain", "--untracked-files=no"])?
                .trim()
                .is_empty(),
        })
    }

    /// The first 12 digits.
    #[must_use]
    pub fn short(&self) -> &str {
        &self.revision[..self.revision.len().min(12)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasets::test_support::TempDir;

    #[test]
    fn fingerprint_from_a_fixture_tree() {
        let dir = TempDir::new("fingerprint");
        let put = |rel: &str, text: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
            std::fs::write(p, text).expect("write");
        };
        let cpuinfo: String = (0..4)
            .map(|p| {
                format!(
                    "processor\t: {p}\nmodel name\t: Test CPU\nphysical id\t: 0\ncore id\t: {}\n\n",
                    p % 2
                )
            })
            .collect();
        put("proc/cpuinfo", &cpuinfo);
        put(
            "proc/meminfo",
            "MemTotal: 8388608 kB\nMemAvailable: 4194304 kB\n",
        );
        put("proc/sys/kernel/osrelease", "6.12.0-test\n");
        put("sys/devices/system/cpu/smt/control", "on\n");
        put(
            "sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
            "performance\n",
        );
        put(
            "sys/devices/system/cpu/cpu1/cpufreq/scaling_governor",
            "powersave\n",
        );
        put(
            "sys/devices/system/cpu/cpu2/cpufreq/scaling_governor",
            "performance\n",
        );
        put(
            "sys/kernel/mm/transparent_hugepage/enabled",
            "always [madvise] never\n",
        );
        let f = Fingerprint::read(&Host::new(dir.path())).expect("readable");
        assert_eq!((f.cores, f.threads), (2, 4));
        assert_eq!(f.memory_bytes, 8 << 30);
        assert_eq!(f.kernel, "6.12.0-test");
        assert_eq!(f.governors, ["performance", "powersave"]);
        assert_eq!(f.smt.as_deref(), Some("on"));
        assert_eq!(f.thp.as_deref(), Some("madvise"));
        assert_eq!(f.boost, None);
        assert_eq!(f.id.len(), 16);
        assert!(
            f.summary().contains("2 cores / 4 threads, 8.0 GiB"),
            "{}",
            f.summary()
        );
    }

    #[test]
    fn the_real_machine_has_a_fingerprint() {
        if cfg!(target_os = "linux") {
            let f = Fingerprint::read(&Host::default()).expect("readable on Linux");
            assert!(f.threads >= 1 && f.memory_bytes > 0);
        }
    }
}
