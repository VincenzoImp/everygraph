// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The harness end to end, through the `everygraph-bench` binary: the official lock and
//! the other refusals, the shipped specs, whole runs under `systemd-run`, eviction, the
//! report.
//!
//! The tests that need `systemd-run --user --scope` (a systemd user session) or a disk
//! that is not tmpfs say so and return early where those are missing (GitHub's Linux
//! runners have both). The official-run tests read the committed specs under
//! `tests/specs/`: an official run refuses a spec that is untracked or changed.
//! The binary is built with the `test-hooks` feature (dev-dependency), which makes it
//! honour `EVERYGRAPH_BENCH_LOCK`, so that no test touches the real lock.

#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]
#![allow(
    clippy::print_stderr,
    reason = "a test that cannot run here says why on standard error"
)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::time::{Duration, Instant};

use everygraph_bench::harness::lock::{LOCK_ENV, OfficialLock};
use everygraph_bench::harness::row::{Row, Status, read_rows};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn test_spec(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/specs")
        .join(format!("{name}.toml"))
        .canonicalize()
        .expect("committed test spec")
        .display()
        .to_string()
}

/// A fresh directory under Cargo's per-target temporary directory (on disk, not tmpfs).
fn scratch(tag: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("harness-{tag}-{}", std::process::id()));
    let _ignored = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("scratch");
    p
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf-8")
}

fn bench(args: &[&str], lock: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_everygraph-bench"))
        .args(args)
        .env(LOCK_ENV, lock)
        .env_remove("EVERYGRAPH_DATA")
        .env_remove("EVERYGRAPH_BENCH_WORK")
        .env_remove("EVERYGRAPH_SCRATCH")
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

/// Held by the tests that read or write hundreds of megabytes, so that one's storage
/// traffic is not another's "others' traffic".
static STORAGE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn storage() -> std::sync::MutexGuard<'static, ()> {
    STORAGE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
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
name = "touch"
group = "basic"
repetitions = 1
# 96 MiB of anonymous memory, touched and held for half a second.
command = ["python3", "-c", "import time; x = bytearray(96 << 20); x[::4096] = b'1' * len(x[::4096]); time.sleep(0.5)"]

[[benchmark]]
name = "oom"
group = "basic"
memory_limit = "32M"
repetitions = 1
command = ["python3", "-c", "x = bytearray(128 << 20); x[::4096] = b'1' * len(x[::4096])"]

[[benchmark]]
name = "left"
group = "basic"
repetitions = 1
shell = "sleep 25 & echo $! > {case_work}/pid"

[[benchmark]]
name = "runner"
group = "basic"
runner = "toy"
repetitions = 1
shell = "echo '{{\"tool\": \"toy\", \"version\": \"1.0\", \"algorithm\": \"echo\", \"options\": {{\"n\": 1}}, \"seconds\": 0.1}}'"
record_json = { tool = "/tool", version = "/version", algorithm = "/algorithm", options = "/options" }
metric = [{ name = "seconds", json = "/seconds", unit = "s" }]

[[benchmark]]
name = "stop"
group = "stop"
repetitions = 1
# A non-interactive shell starts background jobs with SIGINT ignored: the trap ends
# its child itself (and waits for it), or the harness would find it left over.
shell = "trap 'kill $!; wait; sleep 0.2; exit 130' INT; sleep 10 & wait"
interrupt = { after = "300ms" }
reference = [{ metric = "stop_seconds", max = 2.0, source = "ux-contract C-4" }]

[[benchmark]]
name = "stop-badly"
group = "stop"
repetitions = 1
shell = "trap 'kill $!; wait; exit 0' INT; sleep 10 & wait"
interrupt = { after = "300ms" }

[[benchmark]]
name = "later"
repetitions = 1
command = ["true"]
unavailable = "not installable here, for the test"
"#;

fn write_spec(dir: &Path, text: &str) -> PathBuf {
    let p = dir.join("selftest.toml");
    std::fs::write(&p, text).expect("spec");
    p
}

/// A busy neighbour: two shell loops, killed by their handles when dropped.
struct Busy(Vec<Child>);

impl Busy {
    fn start() -> Self {
        Self(
            (0..2)
                .map(|_| {
                    Command::new("sh")
                        .args(["-c", "while :; do :; done"])
                        .spawn()
                        .expect("sh")
                })
                .collect(),
        )
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Acceptance: "starting a second official run while one is active fails fast instead
/// of queueing". The lock is held through the library, as a running official run holds
/// it. The spec's gate would refuse the run anyway (it asks for 1000 TiB), and without
/// --allow-dirty a dirty tree would too: the refusal must name the lock, so the lock is
/// taken before every other check.
#[test]
fn a_second_official_run_fails_fast_and_the_lock_comes_first() {
    let dir = scratch("lock");
    let lock = dir.join("official.lock");
    let _held = OfficialLock::take(&lock, "process 1, spec headline, run 20261125T000000Z-test")
        .expect("free");
    let started = Instant::now();
    let out = bench(
        &[
            "run",
            &test_spec("lockfirst"),
            "--official",
            "--repo",
            s(&repo()),
            "--work",
            s(&dir.join("work")),
            "--out",
            s(&dir.join("rows.jsonl")),
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
fn official_runs_refuse_a_looser_gate_an_untracked_spec_and_a_memory_backed_work_directory() {
    let dir = scratch("refusals");
    let lock = dir.join("official.lock");
    let run = |args: &[&str]| -> (Option<i32>, String) {
        let mut all = vec!["run"];
        all.extend_from_slice(args);
        let (root, rows) = (repo(), dir.join("rows.jsonl"));
        all.extend(["--repo", s(&root), "--out", s(&rows)]);
        let o = bench(&all, &lock);
        (o.status.code(), text(&o.stderr))
    };
    // --max-load above the spec's value.
    let (code, err) = run(&[
        &test_spec("disturbed"),
        "--official",
        "--allow-dirty",
        "--max-load",
        "2000",
        "--work",
        s(&dir.join("w")),
    ]);
    assert_eq!(code, Some(3), "{err}");
    assert!(err.contains("stricter, not looser"), "{err}");
    // A spec that is not committed.
    let spec = write_spec(&dir, SPEC);
    let (code, err) = run(&[
        s(&spec),
        "--official",
        "--allow-dirty",
        "--work",
        s(&dir.join("w")),
    ]);
    assert_eq!(code, Some(3), "{err}");
    assert!(
        err.contains("outside") || err.contains("not tracked"),
        "{err}"
    );
    // A work directory in memory, exploratory runs too, before anything is copied.
    let shm = Path::new("/dev/shm").join(format!("everygraph-bench-test-{}", std::process::id()));
    if Path::new("/dev/shm").is_dir() {
        let (code, err) = run(&[s(&spec), "--only", "echo", "--work", s(&shm)]);
        let _ = std::fs::remove_dir_all(&shm);
        assert_eq!(code, Some(1), "{err}");
        assert!(
            err.contains("tmpfs") && err.contains("EVERYGRAPH_BENCH_WORK"),
            "{err}"
        );
    }
    assert!(!dir.join("rows.jsonl").exists());
}

#[test]
fn every_spec_in_the_repository_passes_the_check() {
    let dir = scratch("check");
    let out = bench(
        &["check", "--repo", s(&repo()), "--data", s(&dir)],
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
fn a_warm_case_below_its_datasets_size_fails_the_lint() {
    let dir = scratch("lint");
    std::fs::create_dir_all(dir.join("data/g")).expect("data");
    std::fs::write(dir.join("data/g/targets.u32"), vec![0_u8; 8192]).expect("dataset");
    let case = |extra: &str| {
        format!(
            "schema = 1\nname = \"selftest\"\ndescription = \"lint\"\n[defaults]\n\
             memory_limit = \"4K\"\n[[benchmark]]\nname = \"scan\"\ndataset_path = \"g\"\n\
             {extra}\ncommand = [\"true\"]\nmetric = [{{ name = \"seconds\", json = \"/s\" }}]\n"
        )
    };
    let check = |extra: &str| {
        let p = write_spec(&dir, &case(extra));
        let data = dir.join("data");
        bench(
            &["check", s(&p), "--repo", s(&repo()), "--data", s(&data)],
            &dir.join("l"),
        )
    };
    let warm = check("");
    assert_eq!(warm.status.code(), Some(1));
    assert!(
        text(&warm.stdout).contains("LINT: memory limit 4 KiB is below the size of g (8 KiB)"),
        "{}",
        text(&warm.stdout)
    );
    for extra in ["cache = \"cold\"", "warm_ok = \"reads with O_DIRECT\""] {
        let out = check(extra);
        assert!(out.status.success(), "{extra}: {}", text(&out.stdout));
    }
}

#[test]
fn a_spec_without_a_memory_limit_is_refused_before_anything_runs() {
    let dir = scratch("nolimit");
    let p = write_spec(&dir, &SPEC.replace("memory_limit = \"256M\"", ""));
    let out = bench(&["check", s(&p), "--repo", s(&repo())], &dir.join("l"));
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
            s(&dir.join("m.json")),
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

fn exploratory(tag: &str, spec_text: &str, extra: &[&str]) -> (Output, Vec<Row>, PathBuf) {
    let dir = scratch(tag);
    let spec = write_spec(&dir, spec_text);
    let rows_path = dir.join("rows.jsonl");
    let (root, work) = (repo(), dir.join("work"));
    let mut args = vec![
        "run",
        s(&spec),
        "--repo",
        s(&root),
        "--work",
        s(&work),
        "--out",
        s(&rows_path),
    ];
    args.extend_from_slice(extra);
    let out = bench(&args, &dir.join("l"));
    let rows = read_rows(&rows_path).unwrap_or_default();
    (out, rows, dir)
}

/// A whole exploratory run: metrics, peaks read after the command, an OOM kill under
/// the limit, leftover processes killed, a runner's declared fields, C-4 stops clean and
/// unclean, an unavailable benchmark, the report.
#[test]
fn an_exploratory_run_records_every_field() {
    if !systemd_run_works() {
        eprintln!("skipped: systemd-run --user --scope does not work here (no user session)");
        return;
    }
    let (out, rows, dir) = exploratory("run", SPEC, &[]);
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    // The OOM, leftover and unclean-stop rows make the run exit 1.
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert_eq!(rows.len(), 9, "{stderr}");
    let row = |b: &str| {
        rows.iter()
            .find(|r| r.benchmark == b)
            .unwrap_or_else(|| panic!("{b}: {stderr}"))
    };

    let echo: Vec<_> = rows.iter().filter(|r| r.benchmark == "echo").collect();
    assert_eq!(echo.len(), 2);
    for r in &echo {
        assert_eq!(r.status, Status::Ok, "{:?}", r.error);
        assert_eq!(r.metrics["seconds"], 0.25);
        assert_eq!(r.metrics["iters"], 2.0);
        assert!(!r.official && r.gate.is_none());
        assert_eq!(r.memory_limit_bytes, 256 << 20);
        let m = r.memory.as_ref().expect("memory");
        assert_eq!(
            m.limit_bytes,
            Some(256 << 20),
            "the kernel applied the limit"
        );
        assert_eq!(m.swap_limit_bytes, Some(0));
        assert!(r.time.expect("time").wall_seconds > 0.0);
        assert_eq!(r.exit.expect("exit").code, Some(0));
        for p in [
            "MemoryMax=268435456",
            "MemorySwapMax=0",
            "OOMPolicy=continue",
        ] {
            assert!(r.wrapper.iter().any(|a| a == p), "{p}");
        }
        assert!(r.load_start.is_some() && r.mem_available_start_bytes.is_some());
        let a = &r.disturbance.activity;
        assert!(a.settle_busy_cores.is_some() && a.settle_io_mb_s.is_some());
        // A run this short gets a settle window after it too.
        assert!(a.after_busy_cores.is_some() && a.after_io_mb_s.is_some());
        assert_eq!(r.git.revision.len(), 40);
    }

    // memory.peak and the sampled anonymous peak are read after the command ran.
    let touch = row("touch");
    assert_eq!(touch.status, Status::Ok, "{:?}", touch.error);
    let m = touch.memory.as_ref().expect("memory");
    assert!(m.peak_bytes >= 96 << 20, "peak {}", m.peak_bytes);
    assert!(
        m.anon_peak_bytes >= 64 << 20,
        "anon peak {}",
        m.anon_peak_bytes
    );
    // The program of an argument list is hashed into the row.
    assert!(
        touch
            .binaries
            .iter()
            .any(|(p, h)| p.ends_with("python3") && h.len() == 64),
        "{:?}",
        touch.binaries
    );

    let oom = row("oom");
    assert_eq!(oom.status, Status::Oom, "{:?}", oom.error);
    assert!(oom.oom.expect("events").oom_kills >= 1);

    // A backgrounded child is found, killed and reported.
    let left = row("left");
    assert_eq!(left.status, Status::LeftProcesses, "{:?}", left.error);
    assert!(!left.left_processes.is_empty());
    let pid: u32 = std::fs::read_to_string(dir.join("work/selftest/cases/left/pid"))
        .expect("pid file")
        .trim()
        .parse()
        .expect("pid");
    assert!(
        left.left_processes.contains(&pid),
        "{:?} vs {pid}",
        left.left_processes
    );
    assert!(!alive(pid), "the leftover sleep {pid} survived");

    let runner = row("runner");
    assert_eq!(runner.status, Status::Ok, "{:?}", runner.error);
    assert_eq!(runner.recorded["tool"], "toy");
    assert_eq!(runner.recorded["algorithm"], "echo");
    assert_eq!(runner.recorded["options"], r#"{"n":1}"#);

    let stop = row("stop");
    assert_eq!(stop.status, Status::Ok, "{:?}", stop.error);
    let st = stop.stop.as_ref().expect("stop record");
    assert_eq!(st.signal, "INT");
    let took = st.stop_seconds.expect("stopped");
    assert!((0.15..2.0).contains(&took), "stop took {took} s");
    let bad = row("stop-badly");
    assert_eq!(
        bad.status,
        Status::Failed,
        "exit 0 after SIGINT is not a clean stop"
    );
    assert!(
        bad.error
            .as_deref()
            .unwrap_or("")
            .contains("a clean stop exits with 130"),
        "{:?}",
        bad.error
    );

    assert_eq!(row("later").status, Status::Unavailable);

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
    assert!(
        stdout.contains("**References: 2 within, 0 off, 0 unchecked.**"),
        "{stdout}"
    );

    let chart = dir.join("chart.json");
    let report = bench(
        &["report", s(&dir.join("rows.jsonl")), "--json", s(&chart)],
        &dir.join("l"),
    );
    assert!(report.status.success(), "{}", text(&report.stderr));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&chart).expect("chart")).expect("json");
    assert!(v["series"].as_array().is_some_and(|s| !s.is_empty()));
}

/// Each cold repetition reads a copy evicted just before it: three repetitions over two
/// copies all read the whole file from storage and pass the residency check (the third
/// reads copy 0 again, which the second repetition's eviction does not touch).
#[test]
fn a_cold_case_is_evicted_before_every_repetition() {
    let _storage = storage();
    if !systemd_run_works() || Command::new("fincore").arg("--version").output().is_err() {
        eprintln!("skipped: needs systemd-run --user and util-linux fincore");
        return;
    }
    let data = scratch("cold-data");
    let file = data.join("graph.bin");
    std::fs::write(&file, vec![7u8; 8 << 20]).expect("data");
    let spec = format!(
        r#"
schema = 1
name = "selftest"
description = "cold"
[defaults]
memory_limit = "256M"
settle = "100ms"
[[benchmark]]
name = "read"
dataset_path = "{}"
cache = "cold"
repetitions = 3
command = ["sh", "-c", "cat {{dataset}} > /dev/null"]
"#,
        file.display()
    );
    let (out, rows, _dir) = exploratory("cold", &spec, &[]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let copies: Vec<u32> = rows.iter().filter_map(|r| r.cache.copy).collect();
    assert_eq!(copies, [0, 1, 0]);
    for r in &rows {
        assert_eq!(r.status, Status::Ok, "{:?}", r.error);
        assert_eq!(r.cache.resident_bytes, Some(0));
        assert!(
            r.io.as_ref().expect("io").read_bytes >= 8 << 20,
            "read from storage: {:?}",
            r.io
        );
        assert_eq!(
            r.dataset.as_ref().expect("dataset").listing_sha256.len(),
            64
        );
    }
}

/// An official run next to a busy neighbour marks its repetition disturbed, says why,
/// and records the gate it passed.
#[test]
fn an_official_run_marks_a_disturbed_repetition() {
    if !systemd_run_works() {
        eprintln!("skipped: systemd-run --user --scope does not work here (no user session)");
        return;
    }
    let dir = scratch("disturbed");
    let busy = Busy::start();
    let out = bench(
        &[
            "run",
            &test_spec("disturbed"),
            "--official",
            "--allow-dirty",
            "--repo",
            s(&repo()),
            "--work",
            s(&dir.join("work")),
            "--out",
            s(&dir.join("rows.jsonl")),
        ],
        &dir.join("official.lock"),
    );
    drop(busy);
    let stderr = text(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    let rows = read_rows(&dir.join("rows.jsonl")).expect("rows");
    let r = &rows[0];
    assert!(r.official);
    assert!(r.disturbance.disturbed, "{:?}", r.disturbance);
    assert!(!r.disturbance.reasons.is_empty());
    let g = r.gate.as_ref().expect("gate record");
    assert_eq!(g.max_load, 1000.0);
    assert!(g.lock_path.ends_with("official.lock"));
    assert_eq!(
        r.spec_path.as_deref(),
        Some("crates/everygraph-bench/tests/specs/disturbed.toml")
    );
    assert!(stderr.contains("DISTURBED"), "{stderr}");
    assert!(
        text(&out.stdout).contains("1 disturbed (excluded)"),
        "{}",
        text(&out.stdout)
    );
}

/// #142: a warm-up that the OOM killer ends writes a row with its whole measurement —
/// status `oom`, the anonymous peak it reached, the OOM events, the exit, I/O — marked
/// as warm-up 1 (repetition 0, never counted), and the repetitions are skipped.
#[test]
fn a_failed_warm_up_writes_its_measurement() {
    if !systemd_run_works() {
        eprintln!("skipped: systemd-run --user --scope does not work here (no user session)");
        return;
    }
    // 1 MiB of written memory every 5 ms, up to 256 MiB, under a 32 MiB limit: the
    // sampler (every 20 ms) sees the anonymous memory grow before the kill.
    let spec = r#"
schema = 1
name = "selftest"
description = "a warm-up OOM-killed under a small limit"
[defaults]
memory_limit = "32M"
settle = "100ms"
[[benchmark]]
name = "grow"
warmup = 1
repetitions = 2
command = ["python3", "-c", "import time\nx = []\nfor _ in range(256):\n    x.append(b'1' * (1 << 20))\n    time.sleep(0.005)"]
"#;
    let (out, rows, dir) = exploratory("warmup-oom", spec, &[]);
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert_eq!(rows.len(), 1, "one row, the repetitions skipped: {stderr}");
    let r = &rows[0];
    assert_eq!(r.status, Status::Oom, "{:?}", r.error);
    assert_eq!((r.repetition, r.warmup), (0, Some(1)));
    assert!(!r.counts());
    let m = r.memory.as_ref().expect("the warm-up's memory is recorded");
    assert_eq!(m.limit_bytes, Some(32 << 20));
    assert!(m.anon_peak_bytes > 0, "anon peak {}", m.anon_peak_bytes);
    assert!(m.peak_bytes >= 16 << 20, "peak {}", m.peak_bytes);
    assert!(r.oom.expect("OOM events").oom_kills >= 1);
    let exit = r.exit.expect("exit");
    assert_eq!(exit.signal, Some(9), "{exit:?}");
    assert!(r.time.expect("time").wall_seconds > 0.0);
    let io = r.io.as_ref().expect("io");
    assert!(io.unknown.is_none(), "{io:?}");
    assert!(!r.command.is_empty() && !r.wrapper.is_empty());
    let e = r.error.as_deref().unwrap_or("");
    assert!(
        e.contains("warm-up run 1 of 1 failed") && e.contains("OOM killer"),
        "{e}"
    );
    assert!(!r.disturbance.disturbed);
    let _ = std::fs::remove_dir_all(dir);
}

/// #142: a command that forks a child doing known reads and is cut by the timeout
/// counts those reads as its own. The scope's SIGKILL orphans the child; the helper,
/// a child subreaper, reaps it before reading `/proc/self/io`. Before, the child was
/// reaped by init, `read_bytes` was near 0, and the 256 MiB were "others' storage
/// traffic during" the run (above the 50 MB/s threshold over 3 s).
#[test]
fn a_timed_out_run_counts_the_reads_of_its_orphaned_children() {
    use everygraph_bench::harness::cache::evict;
    let _storage = storage();
    if !systemd_run_works() {
        eprintln!("skipped: systemd-run --user --scope does not work here (no user session)");
        return;
    }
    let data = scratch("timeout-data");
    let file = data.join("data.bin");
    let size: u64 = 256 << 20;
    std::fs::write(&file, vec![3u8; usize::try_from(size).expect("size")]).expect("data");
    evict(&file).expect("evict");
    // `sh` forks a subshell that reads the file and then sleeps; the timeout kills both.
    let spec = format!(
        r#"
schema = 1
name = "selftest"
description = "a forking reader cut by the timeout"
[defaults]
memory_limit = "128M"
settle = "100ms"
[[benchmark]]
name = "reader"
warmup = 0
repetitions = 1
timeout = "3s"
shell = "sh -c 'cat {} > /dev/null; exec sleep 60' & wait"
"#,
        file.display()
    );
    let (out, rows, dir) = exploratory("timeout-io", &spec, &[]);
    let stderr = text(&out.stderr);
    let _ = std::fs::remove_dir_all(&data);
    assert_eq!(rows.len(), 1, "{stderr}");
    let r = &rows[0];
    assert_eq!(r.status, Status::Timeout, "{:?}", r.error);
    assert!(r.exit.expect("exit").timed_out);
    let io = r.io.as_ref().expect("io");
    assert!(io.unknown.is_none(), "{io:?}");
    assert!(
        io.read_bytes >= size,
        "the orphaned reader's {size} bytes are the run's own: {io:?}"
    );
    let d = &r.disturbance;
    let others = d
        .activity
        .others_io_mb_s_during
        .expect("computed, the I/O is known");
    // Without the fix the reader's 256 MiB were others' traffic (~89 MB/s over the 3 s
    // on the lab machine, above the 50 MB/s threshold). Checked when the machine's
    // storage was quiet just before the run: other work (a build, another test binary)
    // is others' traffic too, and an exploratory row records the reason either way.
    let quiet = d
        .activity
        .settle_io_mb_s
        .is_some_and(|v| v <= d.threshold_io_mb_s / 10.0);
    if quiet {
        assert!(
            !d.reasons
                .iter()
                .any(|x| x.contains("storage traffic during")),
            "others' traffic {others:.2} MB/s: {:?}",
            d.reasons
        );
    } else {
        eprintln!("not checked: storage was busy before the run: {d:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// #142: when the run's own I/O cannot be established — here a descendant moved itself
/// into a scope of its own and is still running when the helper reaps — the row says so,
/// its byte counts are no metric, and others' traffic during the run is not computed
/// (it would otherwise be all the machine's traffic).
#[test]
fn a_run_whose_own_io_is_unknown_says_so() {
    if !systemd_run_works() {
        eprintln!("skipped: systemd-run --user --scope does not work here (no user session)");
        return;
    }
    let spec = r#"
schema = 1
name = "selftest"
description = "a descendant that escapes the scope"
[defaults]
memory_limit = "64M"
settle = "100ms"
[[benchmark]]
name = "escape"
warmup = 0
repetitions = 1
shell = "systemd-run --user --scope --quiet --collect -p MemoryMax=16M -- sleep 20 & echo $! > {case_work}/pid; sleep 1"
"#;
    let (out, rows, dir) = exploratory("io-unknown", spec, &[]);
    let stderr = text(&out.stderr);
    // The escaped sleep, by its process id (systemd-run execs it in place).
    if let Some(pid) = std::fs::read_to_string(dir.join("work/selftest/cases/escape/pid"))
        .ok()
        .and_then(|p| p.trim().parse::<u32>().ok())
    {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    assert_eq!(rows.len(), 1, "{stderr}");
    let r = &rows[0];
    let io = r.io.as_ref().expect("io");
    let why = io.unknown.as_deref().expect("the I/O is marked unknown");
    assert!(why.contains("outside its scope"), "{why}");
    assert_eq!(r.metric("read_bytes"), None);
    assert_eq!(r.disturbance.activity.others_io_mb_s_during, None);
    assert!(r.disturbance.activity.others_cores_during.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn eviction_empties_the_page_cache_of_a_private_file() {
    use everygraph_bench::harness::cache::{MAX_RESIDENT_FRACTION, evict, resident};
    let _storage = storage();
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

#[test]
fn clean_deletes_only_a_marked_work_directory() {
    let dir = scratch("clean");
    let work = dir.join("work");
    std::fs::create_dir_all(work.join("copies")).expect("mkdir");
    std::fs::write(work.join("copies/x"), "x").expect("write");
    let out = bench(
        &["clean", "--repo", s(&repo()), "--work", s(&work)],
        &dir.join("l"),
    );
    assert_eq!(out.status.code(), Some(1), "no marker, nothing deleted");
    assert!(work.join("copies/x").exists());
    std::fs::write(work.join(".everygraph-bench-work"), "").expect("marker");
    let out = bench(
        &["clean", "--repo", s(&repo()), "--work", s(&work)],
        &dir.join("l"),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(!work.exists());
}
