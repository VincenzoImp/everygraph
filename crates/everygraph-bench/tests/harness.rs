// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The harness end to end, through the `everygraph-bench` binary: the official lock, the
//! shipped specs, a whole exploratory run under `systemd-run`, eviction, the report.
//!
//! The tests that need `systemd-run --user --scope` (a systemd user session) or a disk
//! that is not tmpfs say so and return early where those are missing (GitHub's Linux
//! runners have both). The harness runs on Linux only (cgroups v2, `systemd-run`).

#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]
#![allow(
    clippy::print_stderr,
    reason = "a test that cannot run here says why on standard error"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use everygraph_bench::harness::lock::{LOCK_ENV, OfficialLock};
use everygraph_bench::harness::row::{Status, read_rows};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// A fresh directory under Cargo's per-target temporary directory (on disk, not tmpfs).
fn scratch(tag: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("harness-{tag}-{}", std::process::id()));
    let _ignored = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("scratch");
    p
}

fn bench(args: &[&str], lock: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_everygraph-bench"))
        .args(args)
        .env(LOCK_ENV, lock)
        .env_remove("EVERYGRAPH_DATA")
        .env_remove("EVERYGRAPH_BENCH_WORK")
        .output()
        .expect("everygraph-bench runs")
}

fn text(o: &[u8]) -> String {
    String::from_utf8_lossy(o).into_owned()
}

fn systemd_run_works() -> bool {
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

const SPEC: &str = r#"
schema = 1
name = "selftest"
description = "the harness measuring itself"

[defaults]
memory_limit = "256M"
repetitions = 2
warmup = 0
settle = "100ms"

[[benchmark]]
name = "echo"
group = "basic"
shell = "echo noise; echo '{{\"seconds\": 0.25, \"iters\": [3, 1, 2]}}'"
conditions = { graph = "none", precision = "n/a" }
metric = [
  { name = "seconds", json = "/seconds", unit = "s" },
  { name = "iters", json = "/iters", reduce = "median" },
]
reference = [{ metric = "seconds", value = 0.25, source = "this test" }]

[[benchmark]]
name = "oom"
group = "basic"
memory_limit = "32M"
repetitions = 1
command = ["python3", "-c", "x = bytearray(128 << 20); x[::4096] = b'1' * len(x[::4096])"]

[[benchmark]]
name = "stop"
group = "stop"
repetitions = 1
shell = "trap 'sleep 0.2; exit 130' INT; sleep 10 & wait"
interrupt = { after = "300ms" }
reference = [{ metric = "stop_seconds", max = 2.0, source = "ux-contract C-4" }]

[[benchmark]]
name = "later"
repetitions = 1
command = ["true"]
unavailable = "not installable here, for the test"
"#;

fn write_spec(dir: &Path) -> PathBuf {
    let p = dir.join("selftest.toml");
    std::fs::write(&p, SPEC).expect("spec");
    p
}

/// Acceptance: "starting a second official run while one is active fails fast instead
/// of queueing". The lock is held here through the library, as a running official run
/// holds it; the binary must refuse at once, with exit status 3 and the holder named.
#[test]
fn a_second_official_run_fails_fast() {
    let dir = scratch("lock");
    let lock = dir.join("official.lock");
    let spec = write_spec(&dir);
    let _held = OfficialLock::take(&lock, "process 1, spec headline, run 20261125T000000Z-test")
        .expect("free");
    let started = Instant::now();
    let out = bench(
        &[
            "run",
            spec.to_str().expect("utf-8"),
            // No --allow-dirty: the lock is checked before the tree, so a second run
            // learns first that one is in progress.
            "--official",
            "--repo",
            repo().to_str().expect("utf-8"),
            "--work",
            dir.join("work").to_str().expect("utf-8"),
            "--out",
            dir.join("rows.jsonl").to_str().expect("utf-8"),
        ],
        &lock,
    );
    let elapsed = started.elapsed();
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    assert!(
        elapsed < Duration::from_secs(5),
        "took {elapsed:?}: it must not wait"
    );
    assert!(
        stderr.contains("another official run is in progress"),
        "{stderr}"
    );
    assert!(
        stderr.contains("spec headline"),
        "the holder is named: {stderr}"
    );
    assert!(stderr.contains("never queue"), "{stderr}");
    assert!(!dir.join("rows.jsonl").exists(), "nothing was measured");
}

#[test]
fn every_spec_in_the_repository_passes_the_check() {
    let dir = scratch("check");
    let out = bench(
        &[
            "check",
            "--repo",
            repo().to_str().expect("utf-8"),
            "--data",
            dir.to_str().expect("utf-8"),
        ],
        &dir.join("unused.lock"),
    );
    assert!(
        out.status.success(),
        "{}{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    assert!(
        text(&out.stdout).contains("headline: "),
        "{}",
        text(&out.stdout)
    );
}

#[test]
fn a_spec_without_a_memory_limit_is_refused_before_anything_runs() {
    let dir = scratch("nolimit");
    let p = dir.join("selftest.toml");
    std::fs::write(&p, SPEC.replace("memory_limit = \"256M\"", "")).expect("spec");
    let out = bench(
        &[
            "check",
            p.to_str().expect("utf-8"),
            "--repo",
            repo().to_str().expect("utf-8"),
        ],
        &dir.join("l"),
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stdout).contains("has no memory_limit"),
        "{}",
        text(&out.stdout)
    );
}

#[test]
fn measure_refuses_a_cgroup_without_a_memory_limit() {
    // Only meaningful where this test itself runs in a cgroup with no limit.
    let Ok(cg) = std::fs::read_to_string("/proc/self/cgroup") else {
        return;
    };
    let Some(path) = cg.lines().find_map(|l| l.strip_prefix("0::")) else {
        return;
    };
    let max = std::fs::read_to_string(format!("/sys/fs/cgroup{}/memory.max", path.trim()))
        .unwrap_or_default();
    if max.trim() != "max" {
        return;
    }
    let dir = scratch("measure");
    let out = bench(
        &[
            "measure",
            "--report",
            dir.join("m.json").to_str().expect("utf-8"),
            "--timeout-ms",
            "1000",
            "--",
            "true",
        ],
        &dir.join("l"),
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("has no memory limit"),
        "{}",
        text(&out.stderr)
    );
}

/// A whole exploratory run: metrics, an OOM kill under the limit, a C-4 stop, an
/// unavailable benchmark, the results file and the report.
#[test]
fn an_exploratory_run_records_every_field() {
    if !systemd_run_works() {
        eprintln!("skipped: systemd-run --user --scope does not work here (no user session)");
        return;
    }
    let dir = scratch("run");
    let spec = write_spec(&dir);
    let rows_path = dir.join("rows.jsonl");
    let out = bench(
        &[
            "run",
            spec.to_str().expect("utf-8"),
            "--repo",
            repo().to_str().expect("utf-8"),
            "--work",
            dir.join("work").to_str().expect("utf-8"),
            "--out",
            rows_path.to_str().expect("utf-8"),
        ],
        &dir.join("l"),
    );
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    // The OOM row makes the run exit 1; everything else completed.
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    let rows = read_rows(&rows_path).expect("rows");
    assert_eq!(rows.len(), 5, "{stderr}");

    let echo: Vec<_> = rows.iter().filter(|r| r.benchmark == "echo").collect();
    assert_eq!(echo.len(), 2);
    for r in &echo {
        assert_eq!(r.status, Status::Ok, "{:?}", r.error);
        assert_eq!(r.metrics["seconds"], 0.25);
        assert_eq!(r.metrics["iters"], 2.0);
        assert!(!r.official);
        assert_eq!(r.memory_limit_bytes, 256 << 20);
        let m = r.memory.as_ref().expect("memory");
        assert_eq!(
            m.limit_bytes,
            Some(256 << 20),
            "the kernel applied the limit"
        );
        assert_eq!(m.swap_limit_bytes, Some(0));
        assert!(m.peak_bytes > 0);
        assert!(r.time.expect("time").wall_seconds > 0.0);
        assert_eq!(r.exit.expect("exit").code, Some(0));
        assert!(r.wrapper.iter().any(|a| a == "MemoryMax=268435456"));
        assert!(r.wrapper.iter().any(|a| a == "MemorySwapMax=0"));
        assert!(r.wrapper.iter().any(|a| a == "OOMPolicy=continue"));
        assert!(r.load_start.is_some() && r.mem_available_start_bytes.is_some());
        assert!(r.disturbance.settle_busy_cores.is_some());
        assert_eq!(r.git.revision.len(), 40);
    }

    let oom = rows.iter().find(|r| r.benchmark == "oom").expect("oom row");
    assert_eq!(oom.status, Status::Oom, "{:?}", oom.error);
    assert!(oom.oom.expect("events").oom_kills >= 1);

    let stop = rows
        .iter()
        .find(|r| r.benchmark == "stop")
        .expect("stop row");
    assert_eq!(stop.status, Status::Ok, "{:?}", stop.error);
    let s = stop.stop.as_ref().expect("stop record");
    assert_eq!(s.signal, "INT");
    let took = s.stop_seconds.expect("stopped");
    assert!((0.15..2.0).contains(&took), "stop took {took} s");

    let later = rows
        .iter()
        .find(|r| r.benchmark == "later")
        .expect("unavailable row");
    assert_eq!(later.status, Status::Unavailable);

    assert!(stdout.contains("**Exploratory run**"), "{stdout}");
    assert!(stdout.contains("### Not run"), "{stdout}");
    assert!(
        stdout.contains("at most 2.00 s (ux-contract C-4)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("0.250 s (this test): +0.0%, within 10%"),
        "{stdout}"
    );

    let chart = dir.join("chart.json");
    let report = bench(
        &[
            "report",
            rows_path.to_str().expect("utf-8"),
            "--json",
            chart.to_str().expect("utf-8"),
        ],
        &dir.join("l"),
    );
    assert!(report.status.success(), "{}", text(&report.stderr));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&chart).expect("chart")).expect("json");
    assert!(v["series"].as_array().is_some_and(|s| !s.is_empty()));
}

#[test]
fn eviction_empties_the_page_cache_of_a_private_file() {
    use everygraph_bench::harness::cache::{MAX_RESIDENT_FRACTION, evict, resident};
    if Command::new("fincore").arg("--version").output().is_err() {
        eprintln!("skipped: util-linux fincore is not installed");
        return;
    }
    let dir = scratch("evict");
    let f = dir.join("data.bin");
    std::fs::write(&f, vec![1u8; 4 << 20]).expect("write");
    let _ = std::fs::read(&f).expect("read it into the cache");
    let (before, total) = resident(&f).expect("fincore");
    assert_eq!(total, 4 << 20);
    if before == 0 {
        eprintln!("skipped: the file system keeps nothing in the page cache");
        return;
    }
    evict(&f).expect("evict");
    let (after, _) = resident(&f).expect("fincore");
    #[allow(clippy::cast_precision_loss, reason = "test sizes")]
    let share = after as f64 / total as f64;
    assert!(
        share <= MAX_RESIDENT_FRACTION,
        "{after} of {total} bytes still resident"
    );
}
