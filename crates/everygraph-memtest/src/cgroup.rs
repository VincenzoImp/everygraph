// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One operation in a child process inside `systemd-run --user --scope -p
//! MemoryMax=… -p MemorySwapMax=0`, measured by the control group: what only the kernel
//! sees (what the allocator keeps after a free, thread stacks, page tables) and whether
//! the kernel killed it.
//!
//! A scope's control group goes away with its last process, so the child reads its own:
//! it samples `anon` of `memory.stat` while the operation runs (cgroup v2 keeps a peak
//! of the total only, `memory.peak`, which counts the mapped graph file too), then
//! prints a [`Record`]. The parent ([`run`]) starts it, reads the record, and sees a kill
//! in the exit status.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::Duration;

use everygraph_core::Memory;

use crate::ledger::Ledger;
use crate::ops::{Failure, Input, Op};
use crate::probe::alone;

/// The variables that make a test binary the child, and say what it runs.
const OP: &str = "EVERYGRAPH_MEMTEST_OP";
const INPUT: &str = "EVERYGRAPH_MEMTEST_INPUT";
const BUDGET: &str = "EVERYGRAPH_MEMTEST_BUDGET";
const SCRATCH: &str = "EVERYGRAPH_MEMTEST_SCRATCH";

/// The line the child prints its record on.
const MARK: &str = "memtest-record";

/// What a child measured.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    /// `done`, `refused` or `failed`.
    pub outcome: String,
    /// The ledger's anonymous peak.
    pub ledger_peak: u64,
    /// `anon` of the control group just before the operation.
    pub anon_before: u64,
    /// The largest `anon` sampled while it ran (every millisecond).
    pub anon_peak: u64,
    /// `memory.peak`: the largest total since the scope began, page cache included.
    pub memory_peak: u64,
    /// `pagetables`, `kernel` and `shmem` of `memory.stat` at the end.
    pub pagetables: u64,
    /// See [`pagetables`](Self::pagetables).
    pub kernel: u64,
    /// See [`pagetables`](Self::pagetables).
    pub shmem: u64,
    /// `oom_kill` of `memory.events`.
    pub oom_kill: u64,
}

impl Record {
    /// Anonymous memory the operation held at its peak beyond the ledger's peak (the
    /// "untracked memory" of issue #53; negative when the ledger held more).
    #[must_use]
    pub fn gap(&self) -> i64 {
        let s = |b: u64| i64::try_from(b).unwrap_or(i64::MAX);
        s(self.anon_peak) - s(self.anon_before) - s(self.ledger_peak)
    }

    fn line(&self) -> String {
        format!(
            "{MARK} outcome={} ledger_peak={} anon_before={} anon_peak={} memory_peak={} \
             pagetables={} kernel={} shmem={} oom_kill={}",
            self.outcome,
            self.ledger_peak,
            self.anon_before,
            self.anon_peak,
            self.memory_peak,
            self.pagetables,
            self.kernel,
            self.shmem,
            self.oom_kill
        )
    }

    fn parse(line: &str) -> Option<Self> {
        // The test harness may print the test's name first, on the same line.
        let fields: BTreeMap<&str, &str> = line[line.find(MARK)? + MARK.len()..]
            .split_whitespace()
            .filter_map(|kv| kv.split_once('='))
            .collect();
        let n = |k: &str| fields.get(k)?.parse().ok();
        Some(Self {
            outcome: (*fields.get("outcome")?).to_owned(),
            ledger_peak: n("ledger_peak")?,
            anon_before: n("anon_before")?,
            anon_peak: n("anon_peak")?,
            memory_peak: n("memory_peak")?,
            pagetables: n("pagetables")?,
            kernel: n("kernel")?,
            shmem: n("shmem")?,
            oom_kill: n("oom_kill")?,
        })
    }
}

/// Whether `systemd-run --user --scope` works here (a systemd user session).
#[must_use]
pub fn available() -> bool {
    Command::new("systemd-run")
        .args([
            "--user",
            "--scope",
            "--quiet",
            "--collect",
            "-p",
            "MemoryMax=64M",
            "--",
            "true",
        ])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Runs `op` on the input in `input` (written by [`Input::prepare`]) with a budget of
/// `budget` bytes (`None`: unlimited), in this test binary started again as test `test`
/// inside a scope of `memory_max` bytes.
///
/// # Errors
///
/// The child was killed or printed no record: its output.
pub fn run(
    test: &str,
    op: Op,
    input: &Path,
    budget: Option<u64>,
    memory_max: u64,
) -> Result<Record, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let scratch = input.join("child-scratch");
    let out = Command::new("systemd-run")
        .args(["--user", "--scope", "--quiet", "--collect"])
        .arg("-p")
        .arg(format!("MemoryMax={memory_max}"))
        .args(["-p", "MemorySwapMax=0", "--"])
        .arg(exe)
        .args(["--exact", test, "--nocapture", "--test-threads", "1"])
        .env(OP, op.name())
        .env(INPUT, input)
        .env(SCRATCH, &scratch)
        .env(
            BUDGET,
            budget.map_or_else(|| "none".into(), |b| b.to_string()),
        )
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&scratch);
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .find_map(Record::parse)
        .filter(|_| out.status.success())
        .ok_or_else(|| {
            format!(
                "{} with budget {budget:?} under MemoryMax={memory_max}: {}; {stdout}{}",
                op.name(),
                out.status,
                String::from_utf8_lossy(&out.stderr)
            )
        })
}

/// In the child, runs what the parent asked for and prints its [`Record`]; elsewhere
/// does nothing and returns false.
///
/// # Panics
///
/// If the control group cannot be read: the child is only started inside a scope.
pub fn child() -> bool {
    let (Ok(op), Ok(input), Ok(budget), Ok(scratch)) = (
        std::env::var(OP),
        std::env::var(INPUT),
        std::env::var(BUDGET),
        std::env::var(SCRATCH),
    ) else {
        return false;
    };
    let op = Op::named(&op).expect("a known operation");
    let limit = budget.parse().ok();
    let scratch = PathBuf::from(scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("the scratch directory");
    let input = Input::reopen(Path::new(&input));
    let group = own_group();
    let ledger = Arc::new(Ledger::new(limit, everygraph_io::PAGES, None));
    let memory = Memory::from(Arc::clone(&ledger));
    let anon_before = stat(&group).get("anon").copied().unwrap_or(0);
    let peak = AtomicU64::new(anon_before);
    let done = AtomicBool::new(false);
    let result = std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Relaxed) {
                if let Some(&anon) = stat(&group).get("anon") {
                    peak.fetch_max(anon, Relaxed);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let result = alone(|| op.run(&input, &memory, &scratch));
        done.store(true, Relaxed);
        result
    });
    let outcome = match &result {
        Ok(()) => "done",
        Err(Failure::Budget(_)) => "refused",
        Err(Failure::Other(_)) => "failed",
    };
    let stat = stat(&group);
    let get = |k: &str| stat.get(k).copied().unwrap_or(0);
    let events = flat(&group.join("memory.events"));
    let record = Record {
        outcome: outcome.into(),
        ledger_peak: ledger.budget().peak(),
        anon_before,
        anon_peak: peak.into_inner().max(get("anon")),
        memory_peak: read(&group.join("memory.peak")).trim().parse().unwrap_or(0),
        pagetables: get("pagetables"),
        kernel: get("kernel"),
        shmem: get("shmem"),
        oom_kill: events.get("oom_kill").copied().unwrap_or(0),
    };
    #[expect(clippy::print_stdout, reason = "the parent reads the record here")]
    {
        println!("{}", record.line());
        if let Err(e) = result {
            println!("error: {e:?}");
        }
    }
    true
}

/// The control group of this process, from `/proc/self/cgroup` (cgroup v2).
fn own_group() -> PathBuf {
    let text = read(Path::new("/proc/self/cgroup"));
    let path = text
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .expect("a cgroup v2 membership");
    Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn flat(path: &Path) -> BTreeMap<String, u64> {
    read(path)
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(' ')?;
            Some((k.to_owned(), v.trim().parse().ok()?))
        })
        .collect()
}

fn stat(group: &Path) -> BTreeMap<String, u64> {
    flat(&group.join("memory.stat"))
}

#[cfg(test)]
mod tests {
    use super::Record;

    #[test]
    fn a_record_reads_back() {
        let r = Record {
            outcome: "refused".into(),
            ledger_peak: 1,
            anon_before: 2,
            anon_peak: 30,
            memory_peak: 4,
            pagetables: 5,
            kernel: 6,
            shmem: 7,
            oom_kill: 0,
        };
        assert_eq!(Record::parse(&r.line()), Some(r.clone()));
        assert_eq!(r.gap(), 27);
        assert_eq!(Record::parse("other"), None);
    }
}
