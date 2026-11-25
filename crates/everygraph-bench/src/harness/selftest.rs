// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph-bench selftest`: the harness measuring a load it knows, to check on the
//! machine itself, before official runs, that the disturbance check does not see its own
//! work as someone else's.
//!
//! Two benchmarks run as an exploratory spec built here: `cpu`, a busy loop on every
//! logical CPU, and `io`, which writes and syncs 1 GiB. On an idle machine both must find
//! the others' share near zero (at most [`MAX_OTHERS_CORES`] cores and
//! [`MAX_OTHERS_IO_MB_S`] MB/s), and the run's own accounting must see its own work (the
//! cgroup's user time near threads × seconds, the bytes written).

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use super::row::{Row, Status};

/// Others' cores the self-test accepts while the busy loop runs: half the gate's default.
pub const MAX_OTHERS_CORES: f64 = 0.25;

/// Others' storage traffic the self-test accepts while its write runs.
pub const MAX_OTHERS_IO_MB_S: f64 = 10.0;

/// The spec of the self-test: `threads` busy threads for `seconds`, then a 1 GiB write.
#[must_use]
pub fn spec_text(exe: &Path, threads: u32, seconds: u32) -> String {
    format!(
        r#"schema = 1
name = "selftest"
description = "the harness measuring a known load: {threads} busy threads, then a 1 GiB synced write"

[defaults]
memory_limit = "2G"
repetitions = 3
warmup = 0
settle = "1s"

[vars]
exe = '{exe}'

[[benchmark]]
name = "cpu"
threads = [{threads}]
command = ["{{exe}}", "burn", "--threads", "{{threads}}", "--seconds", "{seconds}"]

[[benchmark]]
name = "io"
command = ["{{exe}}", "burn", "--threads", "1", "--seconds", "3", "--write-mib", "1024", "--dir", "{{case_work}}"]
"#,
        exe = exe.display()
    )
}

/// Checks the rows of a self-test run: the verdict text and whether it passed.
#[must_use]
pub fn evaluate(rows: &[Row]) -> (String, bool) {
    let mut out = String::new();
    let mut ok = true;
    let _ = writeln!(
        out,
        "| case | rep | status | wall | own cores | others' cores during | others' MB/s during | before (cores, MB/s) | verdict |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|");
    for r in rows {
        let a = &r.disturbance.activity;
        let wall = r.time.map_or(0.0, |t| t.wall_seconds);
        let own = r
            .time
            .map_or(0.0, |t| t.user_seconds / t.wall_seconds.max(1e-9));
        let mut problems = Vec::new();
        if r.status != Status::Ok {
            problems.push(format!("status {}", r.status.as_str()));
        }
        match r.benchmark.as_str() {
            "cpu" => {
                match a.others_cores_during {
                    Some(c) if c <= MAX_OTHERS_CORES => {}
                    Some(c) => problems.push(format!("others' cores {c:.2} > {MAX_OTHERS_CORES}")),
                    None => problems.push("no reading during the run".into()),
                }
                #[allow(clippy::cast_precision_loss, reason = "thread counts")]
                let threads = f64::from(r.threads);
                if own < 0.9 * threads {
                    problems.push(format!(
                        "the cgroup saw {own:.1} cores of its own, not about {threads}"
                    ));
                }
            }
            "io" => {
                match a.others_io_mb_s_during {
                    Some(v) if v <= MAX_OTHERS_IO_MB_S => {}
                    Some(v) => problems.push(format!(
                        "others' storage traffic {v:.1} MB/s > {MAX_OTHERS_IO_MB_S}"
                    )),
                    None => problems.push("no reading during the run".into()),
                }
                if r.io.as_ref().is_none_or(|i| i.write_bytes < 1 << 30) {
                    problems.push("the run's own 1 GiB write was not counted".into());
                }
            }
            _ => {}
        }
        let fmt = |v: Option<f64>| v.map_or_else(|| "–".into(), |v| format!("{v:.2}"));
        ok &= problems.is_empty();
        let _ = writeln!(
            out,
            "| {} | {} | {} | {wall:.2} s | {own:.2} | {} | {} | {}, {} | {} |",
            r.case,
            r.repetition,
            r.status.as_str(),
            fmt(a.others_cores_during),
            fmt(a.others_io_mb_s_during),
            fmt(a.settle_busy_cores),
            fmt(a.settle_io_mb_s),
            if problems.is_empty() {
                "ok".into()
            } else {
                problems.join("; ")
            }
        );
    }
    let _ = writeln!(
        out,
        "\n{}",
        if ok {
            "Self-test passed: the harness does not count its own work as others'."
        } else {
            "SELF-TEST FAILED: do not start official runs until this is understood (a busy machine also fails it)."
        }
    );
    (out, ok)
}

/// The self-test's load: `threads` threads spinning until `seconds` have passed since
/// the start, after writing and syncing `write_mib` MiB in `dir`, if asked.
///
/// # Errors
///
/// The write fails.
pub fn burn(threads: u32, seconds: u64, write_mib: u64, dir: Option<&Path>) -> Result<(), String> {
    let start = Instant::now();
    if write_mib > 0 {
        let dir = dir.ok_or("--write-mib needs --dir")?;
        let path = dir.join("burn.bin");
        let mut f = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let block = vec![0x5a_u8; 1 << 20];
        for _ in 0..write_mib {
            f.write_all(&block)
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        f.sync_all()
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let deadline = start + Duration::from_secs(seconds);
    let workers: Vec<_> = (0..threads)
        .map(|i| {
            std::thread::spawn(move || {
                let mut x = u64::from(i) | 1;
                while Instant::now() < deadline {
                    for _ in 0..100_000 {
                        x = std::hint::black_box(
                            x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1),
                        );
                    }
                }
                x
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::row::TimeRecord;
    use crate::harness::row::tests::{quiet, sample_row};
    use crate::harness::spec::Spec;

    #[test]
    fn the_selftest_spec_parses() {
        let spec =
            Spec::parse(&spec_text(Path::new("/usr/bin/everygraph-bench"), 32, 10)).expect("valid");
        assert_eq!(spec.cases.len(), 2);
        assert_eq!(spec.cases[0].threads, 32);
    }

    #[test]
    fn the_verdict_needs_quiet_others_and_own_work_seen() {
        let mut cpu = sample_row("cpu", 1, 1.0);
        cpu.benchmark = "cpu".into();
        cpu.threads = 4;
        cpu.time = Some(TimeRecord {
            wall_seconds: 10.0,
            user_seconds: 39.5,
            system_seconds: 0.1,
        });
        cpu.disturbance = quiet();
        cpu.disturbance.activity.others_cores_during = Some(0.05);
        assert!(evaluate(std::slice::from_ref(&cpu)).1);
        let mut busy = cpu.clone();
        busy.disturbance.activity.others_cores_during = Some(1.1);
        let (text, ok) = evaluate(&[busy]);
        assert!(!ok && text.contains("others' cores 1.10"), "{text}");
        let mut blind = cpu;
        blind.time = Some(TimeRecord {
            wall_seconds: 10.0,
            user_seconds: 2.0,
            system_seconds: 0.0,
        });
        assert!(!evaluate(&[blind]).1);
    }

    #[test]
    fn burn_writes_and_spins() {
        let dir = crate::datasets::test_support::TempDir::new("burn");
        burn(2, 0, 2, Some(dir.path())).expect("burn");
        assert_eq!(
            std::fs::metadata(dir.path().join("burn.bin"))
                .expect("file")
                .len(),
            2 << 20
        );
    }
}
