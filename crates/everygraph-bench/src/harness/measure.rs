// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The measuring helper, `everygraph-bench measure`, which runs *inside* the
//! `systemd-run --scope` of one repetition.
//!
//! A scope's cgroup disappears with its last process, so its peak and OOM counters must
//! be read from inside: this helper is the scope's first process, starts the measured
//! command as its child, and reads the cgroup after the child exits. It also samples
//! `memory.stat` while the command runs, because cgroup v2 keeps a peak of the total
//! only (`memory.peak`) and the anonymous/file-backed split is what matters with memory
//! maps (an mmap'd graph shows as RSS but is reclaimable page cache).
//!
//! The helper's own footprint (a few MiB, two idle threads) is inside the cgroup and in
//! its numbers.
//!
//! The run's storage traffic is the helper's own `/proc/self/io`, which adds up the
//! counters of every descendant once it is reaped. A descendant whose parent ends first
//! (a timeout or a stop kills the whole scope at once; a command may also exit before
//! its children) would be reaped by init and its traffic lost, so the helper makes
//! itself a child subreaper (`PR_SET_CHILD_SUBREAPER`): orphans become its children,
//! and it reaps every one of them before it reads the counters. When that cannot be
//! done, [`Measurement::io_unknown`] says why and the traffic of others during the run
//! is not computed from it.

use std::collections::BTreeMap;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::gate::{others_cores, others_io};
use super::procfs::{Cgroup, Host};
use super::spec::{Interrupt, StopSignal};
use super::sys::{self, Signal};

/// What to run and how.
#[derive(Debug, Clone)]
pub struct MeasureOptions {
    /// Program and arguments.
    pub argv: Vec<String>,
    /// `memory.stat` sampling interval.
    pub sample_interval: Duration,
    /// Kill everything in the scope after this long.
    pub timeout: Duration,
    /// A signal to send during the run (ux-contract C-4).
    pub interrupt: Option<Interrupt>,
}

/// How the stop of an interrupted run went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StopRecord {
    /// `INT` or `TERM`.
    pub signal: String,
    /// When it was sent, after the start.
    pub sent_after_seconds: f64,
    /// From the signal to the command's exit; `None` if it had exited before.
    pub stop_seconds: Option<f64>,
}

/// Everything the helper measured, written as JSON for the harness.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    /// The scope's cgroup directory.
    pub cgroup: String,
    /// `memory.max` of the scope (`None`: none, which the harness refuses).
    pub memory_max_bytes: Option<u64>,
    /// `memory.swap.max`.
    pub swap_max_bytes: Option<u64>,
    /// Process id of the command.
    pub pid: Option<u32>,
    /// Start to exit, monotonic clock.
    pub wall_seconds: f64,
    /// User CPU time of everything in the scope (`cpu.stat`).
    pub user_seconds: f64,
    /// System CPU time of everything in the scope.
    pub system_seconds: f64,
    /// Bytes the command and its descendants read from storage (`/proc/self/io` of
    /// the helper after reaping them all, orphans included). A lower bound when
    /// [`Self::io_unknown`] is set.
    pub read_bytes: u64,
    /// Bytes they wrote to storage (or dirtied for it).
    pub write_bytes: u64,
    /// `memory.peak`: largest total (anonymous + file-backed + kernel), exact.
    pub memory_peak_bytes: u64,
    /// Largest `anon` of `memory.stat` seen by sampling.
    pub anon_peak_bytes: u64,
    /// Largest `file` of `memory.stat` (page cache charged to the run) seen by sampling.
    pub file_peak_bytes: u64,
    /// How many samples were taken.
    pub memory_samples: u64,
    /// Page-fault and refault counters of `memory.stat` at the end: the evidence that a
    /// run under a limit below its data's size read from storage (out of core).
    #[serde(default)]
    pub faults: Faults,
    /// The sampling interval in milliseconds.
    pub sample_interval_ms: f64,
    /// `memory.events` at the end (`max`, `oom`, `oom_kill`, …).
    pub memory_events: BTreeMap<String, u64>,
    /// Exit code, if the command exited.
    pub exit_code: Option<i32>,
    /// Signal that ended it, if one did.
    pub signal: Option<i32>,
    /// Killed at the timeout.
    pub timed_out: bool,
    /// The interrupt, if one was sent.
    pub stop: Option<StopRecord>,
    /// Cores used by other processes on the machine while it ran (see
    /// [`super::gate::others_cores`]).
    pub others_cores_during: Option<f64>,
    /// Others' storage traffic while it ran, MB/s ([`super::gate::others_io`]);
    /// `None` when the run's own traffic is unknown.
    pub others_io_mb_s_during: Option<f64>,
    /// Why the run's own storage traffic could not be established (a descendant that
    /// could not be reaped), if it could not. `read_bytes` and `write_bytes` are then a
    /// lower bound.
    #[serde(default)]
    pub io_unknown: Option<String>,
    /// Processes still in the scope when the command exited (a backgrounded child, a
    /// daemon). They were killed with SIGKILL before the scope was read, and the row is
    /// not ok: they would have disturbed the next repetitions.
    pub left_processes: Vec<u32>,
    /// What went wrong in the helper itself, if anything.
    pub error: Option<String>,
}

/// Counters of the scope's `memory.stat`, which starts at zero with the scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Faults {
    /// `pgmajfault`: page faults that waited for storage.
    pub major_faults: u64,
    /// `workingset_refault_file`: file pages read again soon after their eviction.
    pub refaults_file: u64,
    /// `workingset_refault_anon`: the same for anonymous pages (swap).
    pub refaults_anon: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct Peaks {
    anon: u64,
    file: u64,
    samples: u64,
}

fn sample(cg: &Cgroup, peaks: &Mutex<Peaks>) {
    if let Ok(stat) = cg.memory_stat() {
        let mut p = peaks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        p.anon = p.anon.max(stat.get("anon").copied().unwrap_or(0));
        p.file = p.file.max(stat.get("file").copied().unwrap_or(0));
        p.samples += 1;
    }
}

/// Sends `sig` to every process of the cgroup but this one: what a terminal does to a
/// foreground job, limited to the run's own scope. Process ids come from
/// `cgroup.procs`, never from names.
fn signal_scope(cg: &Cgroup, sig: Signal) {
    let me = std::process::id();
    for pid in cg.procs().unwrap_or_default() {
        if pid != me {
            let _ignored = sys::signal(pid, sig);
        }
    }
}

/// After the command's exit: the processes it left in the scope, killed (SIGKILL, by
/// the ids in `cgroup.procs`) until the scope holds the helper alone, for up to 10 s.
fn kill_leftovers(cg: &Cgroup, error: &mut Option<String>) -> Vec<u32> {
    let me = std::process::id();
    let others = |cg: &Cgroup| -> Vec<u32> {
        cg.procs()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| *p != me)
            .collect()
    };
    let left = others(cg);
    if left.is_empty() {
        return left;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        signal_scope(cg, Signal::Kill);
        std::thread::sleep(Duration::from_millis(20));
        let still = others(cg);
        if still.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            error.get_or_insert_with(|| {
                format!(
                    "processes {still:?} survived SIGKILL for 10 s in {}",
                    cg.dir().display()
                )
            });
            break;
        }
    }
    left
}

/// How long [`reap_orphans`] waits for descendants that are still running once the
/// scope is empty (they live in another cgroup, or are just ending).
const REAP_WAIT: Duration = Duration::from_secs(5);

/// After the command and everything left in the scope have ended: reaps every child the
/// helper still has — the descendants orphaned during the run, reparented to it as a
/// subreaper — so that their I/O counters join its own. Must not run while another
/// thread waits for a particular child. `Err` says why the run's I/O is incomplete.
fn reap_orphans() -> Result<(), String> {
    let deadline = Instant::now() + REAP_WAIT;
    loop {
        match sys::reap_one() {
            Ok(sys::Reaped::One) => {}
            Ok(sys::Reaped::None) => return Ok(()),
            Ok(sys::Reaped::Running) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "descendants of the command outside its scope were still running \
                         {} s after it ended; their storage traffic is not counted",
                        REAP_WAIT.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(format!("reaping the command's descendants: {e}")),
        }
    }
}

#[cfg(unix)]
fn signal_of(status: ExitStatus) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(&status)
}

#[cfg(not(unix))]
fn signal_of(_: ExitStatus) -> Option<i32> {
    None
}

fn usec(stat: &BTreeMap<String, u64>, key: &str) -> u64 {
    stat.get(key).copied().unwrap_or(0)
}

/// Runs the command and measures it. Never panics on a failing command: what went
/// wrong is in [`Measurement::error`] and the exit fields.
///
/// # Errors
///
/// The helper is not in a usable cgroup v2 scope, or the scope has no memory limit.
pub fn measure(host: &Host, opts: &MeasureOptions) -> Result<Measurement, String> {
    let cg = Cgroup::new(host.self_cgroup()?);
    let mut m = Measurement {
        cgroup: cg.dir().display().to_string(),
        memory_max_bytes: cg.memory_max()?,
        swap_max_bytes: cg.swap_max().ok().flatten(),
        sample_interval_ms: opts.sample_interval.as_secs_f64() * 1000.0,
        ..Measurement::default()
    };
    if m.memory_max_bytes.is_none() {
        return Err(format!(
            "{} has no memory limit; the helper runs only inside \
             systemd-run --user --scope -p MemoryMax=… (everygraph-bench run does that)",
            m.cgroup
        ));
    }
    let Some((program, args)) = opts.argv.split_first() else {
        return Err("measure needs a command".into());
    };
    // Before the command starts, so that every descendant it orphans comes back here.
    if let Err(e) = sys::become_subreaper() {
        m.io_unknown = Some(format!(
            "cannot become a child subreaper ({e}): descendants orphaned during the run \
             are reaped by init and their storage traffic is not counted"
        ));
    }
    let cpu0 = cg.cpu_stat()?;
    let io0 = host.self_io()?;
    let stat0 = host.cpu_times()?;
    let disk0 = host.disk_bytes()?;

    let peaks = Arc::new(Mutex::new(Peaks::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let (cg, peaks, stop, every) = (
            cg.clone(),
            Arc::clone(&peaks),
            Arc::clone(&stop),
            opts.sample_interval,
        );
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                sample(&cg, &peaks);
                std::thread::sleep(every);
            }
        })
    };

    let start = Instant::now();
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .spawn();
    let (status, end) = match child {
        Err(e) => {
            m.error = Some(format!("cannot start {program:?}: {e}"));
            (None, Instant::now())
        }
        Ok(mut child) => {
            m.pid = Some(child.id());
            let (tx, rx) = mpsc::channel();
            let waiter = std::thread::spawn(move || {
                let status = child.wait();
                let _ignored = tx.send(Instant::now());
                status
            });
            let deadline = start + opts.timeout;
            let mut pending = opts.interrupt;
            let mut sent_at = None;
            let end = loop {
                let now = Instant::now();
                let next = if m.timed_out {
                    // Killed: look again shortly, and kill what may have been forked since.
                    now + Duration::from_millis(100)
                } else {
                    pending.map_or(deadline, |i| deadline.min(start + i.after))
                };
                match rx.recv_timeout(next.saturating_duration_since(now)) {
                    Ok(end) => break end,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break Instant::now(),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                let now = Instant::now();
                if let Some(i) = pending.filter(|i| now >= start + i.after) {
                    let sig = match i.signal {
                        StopSignal::Int => Signal::Int,
                        StopSignal::Term => Signal::Term,
                    };
                    signal_scope(&cg, sig);
                    sent_at = Some(now);
                    m.stop = Some(StopRecord {
                        signal: i.signal.name().into(),
                        sent_after_seconds: (now - start).as_secs_f64(),
                        stop_seconds: None,
                    });
                    pending = None;
                } else if now >= deadline {
                    m.timed_out = true;
                    signal_scope(&cg, Signal::Kill);
                }
            };
            if let (Some(s), Some(at)) = (m.stop.as_mut(), sent_at) {
                s.stop_seconds = Some(end.saturating_duration_since(at).as_secs_f64());
            }
            match waiter.join() {
                Ok(Ok(status)) => (Some(status), end),
                Ok(Err(e)) => {
                    m.error = Some(format!("waiting for the command: {e}"));
                    (None, end)
                }
                Err(_) => {
                    m.error = Some("the waiting thread panicked".into());
                    (None, end)
                }
            }
        }
    };
    let wall = end - start;
    // Machine-wide readings at the command's exit, before anything else is done.
    let stat1 = host.cpu_times()?;
    let disk1 = host.disk_bytes()?;
    m.left_processes = kill_leftovers(&cg, &mut m.error);
    // The command has been waited for and the scope is empty: what is left to reap are
    // orphans, and reaping them adds their I/O to ours.
    if m.io_unknown.is_none() {
        m.io_unknown = reap_orphans().err();
    }
    stop.store(true, Ordering::Relaxed);
    let _ignored = sampler.join();
    sample(&cg, &peaks);

    if let Some(status) = status {
        m.exit_code = status.code();
        m.signal = signal_of(status);
    }
    m.wall_seconds = wall.as_secs_f64();
    let p = *peaks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    (m.anon_peak_bytes, m.file_peak_bytes, m.memory_samples) = (p.anon, p.file, p.samples);
    m.memory_peak_bytes = cg.memory_peak()?;
    if let Ok(stat) = cg.memory_stat() {
        m.faults = Faults {
            major_faults: usec(&stat, "pgmajfault"),
            refaults_file: usec(&stat, "workingset_refault_file"),
            refaults_anon: usec(&stat, "workingset_refault_anon"),
        };
    }
    m.memory_events = cg.memory_events()?;
    let cpu1 = cg.cpu_stat()?;
    #[allow(
        clippy::cast_precision_loss,
        reason = "microsecond counts far below 2^53"
    )]
    let secs = |key: &str| usec(&cpu1, key).saturating_sub(usec(&cpu0, key)) as f64 / 1e6;
    m.user_seconds = secs("user_usec");
    m.system_seconds = secs("system_usec");
    let io1 = host.self_io()?;
    m.read_bytes = io1.read_bytes.saturating_sub(io0.read_bytes);
    m.write_bytes = io1.write_bytes.saturating_sub(io0.write_bytes);
    m.others_cores_during =
        others_cores(stat0, stat1, Duration::from_secs_f64(m.user_seconds), wall);
    // Without the run's own traffic, the machine's would all be counted as others'.
    m.others_io_mb_s_during = if m.io_unknown.is_some() {
        None
    } else {
        others_io(disk0, disk1, m.read_bytes + m.write_bytes, wall)
    };
    Ok(m)
}
