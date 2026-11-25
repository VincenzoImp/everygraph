// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph-bench run`: the order of a run, and one repetition.
//!
//! 1. Select the cases.
//! 2. Official runs: take the lock (fail fast, before anything else), then refuse a
//!    dirty tree, a spec file that is untracked or changed, and a `--max-load` above the
//!    spec's.
//! 3. Find the datasets; refuse a work directory (or, for cold cases, a dataset) on a
//!    memory-backed file system.
//! 4. Official runs: the idle gate (load, `MemAvailable`).
//! 5. Check that `systemd-run --user --scope` works (and `fincore`, for cold cases).
//! 6. Preparation steps, then the private copies of cold cases.
//! 7. Each case: warm-up runs, then each repetition: clean work directory, eviction
//!    (cold), settle window, eviction check, the command under
//!    `systemd-run --scope -p MemoryMax=… -p MemorySwapMax=0 everygraph-bench measure …`,
//!    a settle window after short runs, metrics, one row appended to the results file.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::datasets::{DataRoot, Registry};

use super::cache::{self, Copies, MAX_RESIDENT_FRACTION};
use super::extract::{extract, last_json_object};
use super::gate::{
    self, Activity, MIN_DURING_WINDOW, StartReadings, busy_cores, classify, io_mb_s,
};
use super::lock::{LockError, OfficialLock};
use super::machine::{Fingerprint, GitInfo};
use super::measure::Measurement;
use super::procfs::Host;
use super::row::{
    CacheRecord, DatasetRecord, ExitRecord, GateRecord, IoRecord, MemoryRecord, OomRecord,
    ROW_SCHEMA, ReferenceRecord, Row, RowWriter, Status, TimeRecord, estimate_errors,
    parse_estimate,
};
use super::spec::{CacheMode, Case, DatasetRef, PLAN_QUANTITIES, Prepare, Program, Spec};
use super::template::Scope;
use super::units::{compact_timestamp, human_size, rfc3339, unix_now};

/// Where things are.
#[derive(Debug, Clone)]
pub struct Context {
    /// The repository: `{repo}`, and where results go.
    pub repo: PathBuf,
    /// The data root: `{data}`.
    pub data: DataRoot,
    /// The dataset registry.
    pub registry: Registry,
    /// The work directory: `{work}`, cold copies, logs.
    pub work: PathBuf,
    /// Where `/proc` and `/sys` are.
    pub host: Host,
    /// The `everygraph-bench` executable that runs `measure` inside each scope.
    pub exe: PathBuf,
}

/// Options of one run.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Lock, idle gate, committed results.
    pub official: bool,
    /// Only cases whose id contains one of these.
    pub filters: Vec<String>,
    /// Override of every case's repetitions.
    pub repetitions: Option<u32>,
    /// Override of the gate's load threshold.
    pub max_load: Option<f64>,
    /// Let an official run start from a dirty tree (recorded in every row).
    pub allow_dirty: bool,
    /// Stop after the preparation steps and cold copies.
    pub prepare_only: bool,
    /// Do not run the preparation steps.
    pub skip_prepare: bool,
    /// Results file instead of the default.
    pub out: Option<PathBuf>,
    /// The official lock file.
    pub lock_path: PathBuf,
}

/// How a run ended.
#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    /// The results file (none for `--prepare-only`).
    pub results: Option<PathBuf>,
    /// Rows written.
    pub rows: usize,
    /// Rows that did not complete (failed, timeout, OOM, not cold, harness error).
    pub failed: usize,
    /// Rows marked disturbed.
    pub disturbed: usize,
    /// Benchmarks recorded as unavailable.
    pub unavailable: usize,
}

/// Why a run did not complete.
#[derive(Debug)]
pub enum RunError {
    /// Refused before anything ran: lock held, machine busy, dirty tree.
    Refused(String),
    /// Something failed.
    Failed(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Refused(m) | RunError::Failed(m) => f.write_str(m),
        }
    }
}

impl From<String> for RunError {
    fn from(m: String) -> Self {
        RunError::Failed(m)
    }
}

/// A file name for a case id: `kernels/algo=pr32/t32` → `kernels__algo-pr32__t32`.
#[must_use]
pub fn path_safe(id: &str) -> String {
    id.replace('/', "__")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Where a case's data is.
///
/// # Errors
///
/// A registry name that is not registered.
pub fn dataset_path(ctx: &Context, d: &DatasetRef) -> Result<PathBuf, String> {
    match d {
        DatasetRef::Registry(name) => {
            let ds = ctx.registry.get(name).ok_or_else(|| {
                format!("dataset {name:?} is not in the registry (everygraph-bench list)")
            })?;
            Ok(ctx.data.dataset_dir(ds))
        }
        DatasetRef::Path(p) => {
            let p = Path::new(p);
            Ok(if p.is_absolute() {
                p.to_path_buf()
            } else {
                ctx.data.path().join(p)
            })
        }
    }
}

/// The `systemd-run` prefix that puts a command under a hard memory limit, in a scope of
/// its own (its cgroup) so that its peak and OOM events can be read.
///
/// `OOMPolicy=continue`: by default systemd stops the whole scope when the kernel's OOM
/// killer ends one of its processes, which would kill the measuring helper before it
/// records the OOM (seen in tests: SIGTERM, no report).
#[must_use]
pub fn scope_prefix(memory_limit: u64, description: &str) -> Vec<String> {
    [
        "systemd-run",
        "--user",
        "--scope",
        "--quiet",
        "--collect",
        &format!("--description={description}"),
        "-p",
        &format!("MemoryMax={memory_limit}"),
        "-p",
        "MemorySwapMax=0",
        "-p",
        "OOMPolicy=continue",
        "--",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text.trim();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].trim()
}

/// Checks that hard memory limits work here.
fn preflight(cold: bool) -> Result<(), String> {
    let mut cmd = scope_prefix(64 << 20, "everygraph-bench preflight");
    cmd.push("true".into());
    let out = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run systemd-run ({e}); the harness needs systemd's user manager to put every run under a hard memory limit"))?;
    if !out.status.success() {
        return Err(format!(
            "systemd-run --user --scope does not work here ({}); the harness needs a \
             systemd user session to put every run under a hard memory limit",
            tail(&String::from_utf8_lossy(&out.stderr), 400)
        ));
    }
    if cold && Command::new("fincore").arg("--version").output().is_err() {
        return Err(
            "cold-cache cases check eviction with fincore (mincore), which is missing: \
             install util-linux (Debian and Ubuntu: apt install util-linux)"
                .into(),
        );
    }
    Ok(())
}

fn program_argv(scope: &Scope, p: &Program) -> Result<Vec<String>, String> {
    match p {
        Program::Argv(a) => a.iter().map(|x| scope.expand(x)).collect(),
        Program::Shell(s) => Ok(vec!["sh".into(), "-c".into(), scope.expand(s)?]),
    }
}

/// The file a program name runs: itself when it names a directory, else the first match
/// on `PATH`.
fn find_program(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        return Some(PathBuf::from(program));
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(program))
            .find(|p| p.is_file())
    })
}

/// The text fields a case records: its `record` templates, and its `record_json`
/// pointers into the last JSON object of standard output. The second value lists what
/// could not be read.
fn recorded_fields(
    case: &Case,
    scope: &Scope,
    stdout: &str,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut errors = Vec::new();
    for (k, t) in &case.record {
        match scope.expand(t) {
            Ok(v) => {
                out.insert(k.clone(), v);
            }
            Err(e) => errors.push(format!("record.{k}: {e}")),
        }
    }
    if !case.record_json.is_empty() {
        let doc = last_json_object(stdout);
        for (k, pointer) in &case.record_json {
            match doc.as_ref().and_then(|d| d.pointer(pointer)) {
                Some(serde_json::Value::String(v)) => {
                    out.insert(k.clone(), v.clone());
                }
                Some(v) => {
                    out.insert(k.clone(), v.to_string());
                }
                None => errors.push(format!(
                    "record_json.{k}: the command printed nothing at {pointer:?} (a runner \
                     must say what it ran)"
                )),
            }
        }
    }
    (out, errors)
}

/// Refuses a directory on a memory-backed file system: copies and outputs there would sit
/// in RAM outside every memory limit, and eviction cannot work.
fn refuse_memory_backed(what: &str, path: &Path, fix: &str) -> Result<(), String> {
    match super::sys::memory_backed(path) {
        Ok(Some(fs)) => Err(format!(
            "{what} {} is on {fs}, a file system held in memory: what the harness writes \
             there stays in RAM outside every memory limit, and cold-cache eviction cannot \
             work. {fix}",
            path.display()
        )),
        Ok(None) => Ok(()),
        Err(e) => Err(format!(
            "checking the file system of {}: {e}",
            path.display()
        )),
    }
}

/// The run in progress.
struct Runner<'a> {
    spec: &'a Spec,
    ctx: &'a Context,
    opts: &'a RunOptions,
    machine: Fingerprint,
    git: GitInfo,
    run_id: String,
    log: &'a mut dyn Write,
    copies: BTreeMap<PathBuf, Copies>,
    /// The gate as applied (official runs).
    gate: Option<GateRecord>,
    /// The spec file relative to the repository.
    spec_path: Option<String>,
    /// SHA-256 by (path, size, modification time) of the binaries hashed so far.
    hashes: BTreeMap<(PathBuf, u64, u128), String>,
    /// Listing digests of the datasets, by path.
    listings: BTreeMap<PathBuf, String>,
}

/// One execution of a case's command.
struct Execution {
    measurement: Option<Measurement>,
    stdout: String,
    stderr: String,
    command: Vec<String>,
    wrapper: Vec<String>,
    cwd: PathBuf,
    env: BTreeMap<String, String>,
    error: Option<String>,
}

macro_rules! say {
    ($w:expr, $($arg:tt)*) => {{
        let _ignored = writeln!($w, $($arg)*);
    }};
}

impl Runner<'_> {
    fn base_scope(&self) -> Scope {
        let mut s = Scope::new(self.spec.vars.clone());
        s.set("repo", self.ctx.repo.display().to_string());
        s.set("data", self.ctx.data.path().display().to_string());
        s.set("work", self.ctx.work.display().to_string());
        s.set("spec", self.spec.name.clone());
        s
    }

    fn spec_work(&self) -> PathBuf {
        self.ctx.work.join(&self.spec.name)
    }

    /// `{case_work}`: emptied before each run of the case when `clean_work` is set.
    fn case_work(&self, case: &Case) -> PathBuf {
        self.spec_work().join("cases").join(path_safe(&case.id))
    }

    fn prepare(&mut self, step: &Prepare) -> Result<(), String> {
        let scope = self.base_scope();
        if let Some(c) = &step.creates {
            let p = PathBuf::from(scope.expand(c)?);
            if p.exists() {
                say!(
                    self.log,
                    "prepare {}: skipped, {} exists",
                    step.name,
                    p.display()
                );
                return Ok(());
            }
        }
        let argv = program_argv(&scope, &step.program)?;
        let cwd = match &step.cwd {
            Some(c) => PathBuf::from(scope.expand(c)?),
            None => self.spec_work(),
        };
        let logs = self.spec_work().join("prepare");
        std::fs::create_dir_all(&logs).map_err(|e| format!("creating {}: {e}", logs.display()))?;
        std::fs::create_dir_all(&cwd).map_err(|e| format!("creating {}: {e}", cwd.display()))?;
        let log_path = logs.join(format!("{}.log", path_safe(&step.name)));
        let log_file =
            File::create(&log_path).map_err(|e| format!("creating {}: {e}", log_path.display()))?;
        let err_file = log_file.try_clone().map_err(|e| e.to_string())?;
        let wrapper = scope_prefix(
            step.memory_limit,
            &format!("everygraph-bench prepare {}", step.name),
        );
        say!(
            self.log,
            "prepare {}: {} (limit {}, log {})",
            step.name,
            argv.join(" "),
            human_size(step.memory_limit),
            log_path.display()
        );
        let started = Instant::now();
        let status = Command::new(&wrapper[0])
            .args(&wrapper[1..])
            .args(&argv)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(log_file)
            .stderr(err_file)
            .status()
            .map_err(|e| format!("prepare {}: cannot start systemd-run: {e}", step.name))?;
        if !status.success() {
            let text = std::fs::read_to_string(&log_path).unwrap_or_default();
            return Err(format!(
                "prepare step {} failed ({status}); the end of {}:\n{}",
                step.name,
                log_path.display(),
                tail(&text, 2000)
            ));
        }
        say!(
            self.log,
            "prepare {}: done in {:.1} s",
            step.name,
            started.elapsed().as_secs_f64()
        );
        Ok(())
    }

    fn case_scope(
        &self,
        case: &Case,
        rep: &str,
        dataset: Option<&Path>,
        case_work: &Path,
    ) -> Scope {
        let mut s = self.base_scope();
        s.set("case_work", case_work.display().to_string());
        s.set("threads", case.threads.to_string());
        s.set("rep", rep);
        s.set("benchmark", case.benchmark.clone());
        // Unset {dataset} is an error when used, like any unknown name.
        if let Some(d) = dataset {
            s.set("dataset", d.display().to_string());
        }
        for (k, v) in &case.vars {
            s.set(k, v.clone());
        }
        s
    }

    fn execute(
        &mut self,
        case: &Case,
        rep: &str,
        dataset: Option<&Path>,
    ) -> Result<Execution, String> {
        let case_work = self.case_work(case);
        if case.clean_work && case_work.exists() {
            std::fs::remove_dir_all(&case_work)
                .map_err(|e| format!("emptying {}: {e}", case_work.display()))?;
        }
        std::fs::create_dir_all(&case_work)
            .map_err(|e| format!("creating {}: {e}", case_work.display()))?;
        let scope = self.case_scope(case, rep, dataset, &case_work);
        let command = program_argv(&scope, &case.program)?;
        let cwd = match &case.cwd {
            Some(c) => PathBuf::from(scope.expand(c)?),
            None => case_work.clone(),
        };
        let env: BTreeMap<String, String> = case
            .env
            .iter()
            .map(|(k, v)| scope.expand(v).map(|v| (k.clone(), v)))
            .collect::<Result<_, _>>()?;
        let logs = self.spec_work().join("logs").join(path_safe(&case.id));
        std::fs::create_dir_all(&logs).map_err(|e| format!("creating {}: {e}", logs.display()))?;
        let out_path = logs.join(format!("{rep}.out"));
        let err_path = logs.join(format!("{rep}.err"));
        let report = logs.join(format!("{rep}.measure.json"));
        let _ignored = std::fs::remove_file(&report);
        let open = |p: &Path| File::create(p).map_err(|e| format!("creating {}: {e}", p.display()));

        let mut wrapper = scope_prefix(
            case.memory_limit,
            &format!("everygraph-bench {} {} {rep}", self.spec.name, case.id),
        );
        wrapper.extend([
            self.ctx.exe.display().to_string(),
            "measure".into(),
            "--report".into(),
            report.display().to_string(),
            "--sample-interval-ms".into(),
            case.sample_interval.as_millis().to_string(),
            "--timeout-ms".into(),
            case.timeout.as_millis().to_string(),
        ]);
        if let Some(i) = case.interrupt {
            wrapper.extend([
                "--interrupt-after-ms".into(),
                i.after.as_millis().to_string(),
                "--interrupt-signal".into(),
                i.signal.name().into(),
            ]);
        }
        wrapper.push("--".into());
        let status = Command::new(&wrapper[0])
            .args(&wrapper[1..])
            .args(&command)
            .current_dir(&cwd)
            .envs(&env)
            .stdin(Stdio::null())
            .stdout(open(&out_path)?)
            .stderr(open(&err_path)?)
            .status();
        let stdout =
            String::from_utf8_lossy(&std::fs::read(&out_path).unwrap_or_default()).into_owned();
        let stderr =
            String::from_utf8_lossy(&std::fs::read(&err_path).unwrap_or_default()).into_owned();
        let measurement = std::fs::read_to_string(&report)
            .ok()
            .and_then(|t| serde_json::from_str::<Measurement>(&t).ok());
        let error = match (&status, &measurement) {
            (Err(e), _) => Some(format!("cannot start systemd-run: {e}")),
            (Ok(s), None) => Some(format!(
                "the measuring helper wrote no report ({s}); its standard error ends with: {}",
                tail(&stderr, 1500)
            )),
            (Ok(_), Some(m)) => m.error.clone(),
        };
        // `wrapper` ends with the `--` before the command, as rows record it.
        Ok(Execution {
            measurement,
            stdout,
            stderr,
            command,
            wrapper,
            cwd,
            env,
            error,
        })
    }

    fn base_row(&self, case: &Case, repetition: u32, repetitions: u32) -> Row {
        Row {
            schema: ROW_SCHEMA,
            run_id: self.run_id.clone(),
            spec: self.spec.name.clone(),
            spec_sha256: self.spec.sha256.clone(),
            spec_path: self.spec_path.clone(),
            official: self.opts.official,
            case: case.id.clone(),
            benchmark: case.benchmark.clone(),
            group: case.group.clone(),
            vars: case.vars.clone(),
            runner: case.runner.clone(),
            repetition,
            repetitions,
            status: Status::Ok,
            error: None,
            started_at: rfc3339(unix_now()),
            machine: self.machine.clone(),
            git: self.git.clone(),
            command: Vec::new(),
            binaries: BTreeMap::new(),
            gate: self.gate.clone(),
            wrapper: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            dataset: None,
            conditions: BTreeMap::new(),
            threads: case.threads,
            memory_limit_bytes: case.memory_limit,
            cache: CacheRecord {
                mode: case.cache,
                copy: None,
                resident_bytes: None,
            },
            load_start: None,
            load_end: None,
            mem_available_start_bytes: None,
            disturbance: classify(Activity::default(), &self.spec.gate, self.opts.official),
            time: None,
            io: None,
            memory: None,
            oom: None,
            exit: None,
            stop: None,
            left_processes: Vec::new(),
            recorded: BTreeMap::new(),
            metrics: BTreeMap::new(),
            units: case
                .metrics
                .iter()
                .filter_map(|m| m.unit.clone().map(|u| (m.name.clone(), u)))
                .collect(),
            primary: case.primary.clone(),
            references: case
                .references
                .iter()
                .map(|r| ReferenceRecord {
                    metric: r.metric.clone(),
                    value: r.value,
                    max: r.max,
                    tolerance: r.tolerance,
                    source: r.source.clone(),
                })
                .collect(),
            estimate: None,
            actual: None,
            estimate_error: None,
        }
    }

    fn conditions(&self, case: &Case, scope: &Scope) -> BTreeMap<String, String> {
        case.conditions
            .iter()
            .map(|(k, v)| (k.clone(), scope.expand(v).unwrap_or_else(|_| v.clone())))
            .collect()
    }

    /// Settle window: write back what earlier runs left dirty (an ingest leaves
    /// gigabytes), then measure busy cores and storage traffic while nothing of ours
    /// runs.
    fn settle(&self, case: &Case) -> (Option<f64>, Option<f64>) {
        let _ignored = super::sys::sync_all();
        let host = &self.ctx.host;
        let (cpu0, disk0) = (host.cpu_times().ok(), host.disk_bytes().ok());
        let t = Instant::now();
        std::thread::sleep(case.settle);
        let elapsed = t.elapsed();
        let cores = cpu0
            .zip(host.cpu_times().ok())
            .map(|(a, b)| busy_cores(a, b, elapsed));
        let io = disk0
            .zip(host.disk_bytes().ok())
            .map(|(a, b)| io_mb_s(a, b, elapsed));
        (cores, io)
    }

    /// SHA-256 of a file, cached by path, size and modification time.
    fn hash(&mut self, path: &Path) -> Result<String, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let key = (path.to_path_buf(), meta.len(), mtime);
        if let Some(h) = self.hashes.get(&key) {
            return Ok(h.clone());
        }
        let h = crate::datasets::hash_file(path, false)
            .map_err(|e| e.to_string())?
            .sha256;
        self.hashes.insert(key, h.clone());
        Ok(h)
    }

    /// The binaries a case runs: the declared ones, and the program itself when the
    /// command is an argument list (found on `PATH` if it has no directory).
    fn binaries(
        &mut self,
        case: &Case,
        scope: &Scope,
        argv: &[String],
    ) -> Result<BTreeMap<String, String>, String> {
        let mut files: Vec<PathBuf> = case
            .binaries
            .iter()
            .map(|b| scope.expand(b).map(PathBuf::from))
            .collect::<Result<_, _>>()?;
        if let (Program::Argv(_), Some(first)) = (&case.program, argv.first()) {
            files.extend(find_program(first));
        }
        let mut out = BTreeMap::new();
        for f in files {
            let h = self
                .hash(&f)
                .map_err(|e| format!("hashing a binary of the case: {e}"))?;
            out.insert(f.display().to_string(), h);
        }
        Ok(out)
    }

    fn listing(&mut self, source: &Path) -> String {
        if let Some(l) = self.listings.get(source) {
            return l.clone();
        }
        let l = cache::listing_sha256(source).unwrap_or_default();
        self.listings.insert(source.to_path_buf(), l.clone());
        l
    }

    fn repetition(
        &mut self,
        case: &Case,
        source: Option<&Path>,
        rep: u32,
        reps: u32,
    ) -> Result<Row, String> {
        let mut row = self.base_row(case, rep, reps);
        let rep_name = format!("rep{rep}");
        // Data: the shared path (warm) or the next private copy (cold), evicted.
        let mut data = source.map(Path::to_path_buf);
        if case.cache == CacheMode::Cold {
            let src = source.ok_or("a cold case without a dataset")?;
            let copies = self
                .copies
                .get(src)
                .ok_or_else(|| format!("no cold copies of {}", src.display()))?;
            let (k, copy) = copies.next(case.cold_copies)?;
            cache::evict(&copy)?;
            row.cache.copy = Some(k);
            data = Some(copy);
        }
        let mut activity = Activity::default();
        (activity.settle_busy_cores, activity.settle_io_mb_s) = self.settle(case);
        if let (Some(d), Some(src)) = (&data, source) {
            row.dataset = Some(DatasetRecord {
                name: case
                    .dataset
                    .as_ref()
                    .map_or_else(String::new, |d| d.label().to_string()),
                path: d.display().to_string(),
                bytes: cache::size_of(d).unwrap_or(0),
                listing_sha256: self.listing(src),
            });
        }
        if case.cache == CacheMode::Cold {
            let d = data.as_deref().ok_or("a cold case without a dataset")?;
            let (resident, total) = cache::resident(d)?;
            row.cache.resident_bytes = Some(resident);
            #[allow(clippy::cast_precision_loss, reason = "a share of bytes")]
            let share = if total == 0 {
                0.0
            } else {
                resident as f64 / total as f64
            };
            if share > MAX_RESIDENT_FRACTION {
                row.status = Status::NotCold;
                row.error = Some(format!(
                    "{} of {} still in the page cache after eviction ({:.1}%); is the work \
                     directory on tmpfs, or is the copy mapped by another process?",
                    human_size(resident),
                    human_size(total),
                    share * 100.0
                ));
                return Ok(row);
            }
        }
        row.load_start = self.ctx.host.loadavg().ok();
        row.mem_available_start_bytes = self.ctx.host.mem_available().ok();
        let exec = self.execute(case, &rep_name, data.as_deref())?;
        row.load_end = self.ctx.host.loadavg().ok();
        let short = exec
            .measurement
            .as_ref()
            .is_none_or(|m| m.wall_seconds < MIN_DURING_WINDOW.as_secs_f64());
        if short {
            // Too short to see others while it ran: look just after it as well.
            (activity.after_busy_cores, activity.after_io_mb_s) = self.settle(case);
        }
        let case_work = self.case_work(case);
        let scope = self.case_scope(case, &rep_name, data.as_deref(), &case_work);
        row.conditions = self.conditions(case, &scope);
        row.binaries = self.binaries(case, &scope, &exec.command)?;
        row.command = exec.command;
        row.wrapper = exec.wrapper;
        row.cwd = Some(exec.cwd.display().to_string());
        row.env = exec.env;

        let Some(m) = exec.measurement else {
            row.status = Status::HarnessError;
            row.error = exec.error;
            row.disturbance = classify(activity, &self.spec.gate, self.opts.official);
            return Ok(row);
        };
        activity.others_cores_during = m.others_cores_during;
        activity.others_io_mb_s_during = m.others_io_mb_s_during;
        row.disturbance = classify(activity, &self.spec.gate, self.opts.official);
        row.left_processes.clone_from(&m.left_processes);
        row.time = Some(TimeRecord {
            wall_seconds: m.wall_seconds,
            user_seconds: m.user_seconds,
            system_seconds: m.system_seconds,
        });
        row.io = Some(IoRecord {
            read_bytes: m.read_bytes,
            write_bytes: m.write_bytes,
        });
        row.memory = Some(MemoryRecord {
            limit_bytes: m.memory_max_bytes,
            swap_limit_bytes: m.swap_max_bytes,
            peak_bytes: m.memory_peak_bytes,
            anon_peak_bytes: m.anon_peak_bytes,
            file_peak_bytes: m.file_peak_bytes,
            samples: m.memory_samples,
            sample_interval_ms: m.sample_interval_ms,
        });
        let event = |k: &str| m.memory_events.get(k).copied().unwrap_or(0);
        row.oom = Some(OomRecord {
            max_events: event("max"),
            oom_events: event("oom"),
            oom_kills: event("oom_kill"),
        });
        row.exit = Some(ExitRecord {
            code: m.exit_code,
            signal: m.signal,
            timed_out: m.timed_out,
        });
        row.stop = m.stop.clone();

        let (metrics, mut metric_errors) = extract(&case.metrics, &exec.stdout, &exec.stderr);
        row.metrics = metrics;
        let (recorded, record_errors) = recorded_fields(case, &scope, &exec.stdout);
        row.recorded = recorded;
        metric_errors.extend(record_errors);
        if let Some(plan) = &case.plan {
            self.plan_fields(&mut row, plan, &exec.stdout);
        }
        // A stopped run must end the way a clean stop does (C-7: 128 + the signal).
        let expected_stop = case.interrupt.zip(m.stop.as_ref()).and_then(|(i, s)| {
            s.stop_seconds?;
            Some(i.expect_exit)
        });
        let clean_exit = match expected_stop {
            Some(code) => m.exit_code == Some(code) || m.signal == Some(code - 128),
            None => m.exit_code == Some(0),
        };
        let mut problems: Vec<String> = Vec::new();
        row.status = if exec.error.is_some() {
            problems.extend(exec.error.clone());
            Status::HarnessError
        } else if event("oom_kill") > 0 {
            problems.push(format!(
                "the OOM killer ended {} process(es) of the run at the {} limit",
                event("oom_kill"),
                human_size(case.memory_limit)
            ));
            Status::Oom
        } else if m.timed_out {
            problems.push(format!(
                "killed after the timeout of {} s",
                case.timeout.as_secs()
            ));
            Status::Timeout
        } else if !m.left_processes.is_empty() {
            problems.push(format!(
                "the command left processes {:?} running in its scope; they were killed",
                m.left_processes
            ));
            Status::LeftProcesses
        } else if !clean_exit {
            let ending = match (m.exit_code, m.signal) {
                (Some(c), _) => format!("exit status {c}"),
                (None, Some(s)) => format!("killed by signal {s}"),
                (None, None) => "ended without a status".into(),
            };
            problems.push(match expected_stop {
                Some(code) => {
                    format!("{ending} after the interrupt; a clean stop exits with {code}")
                }
                None => ending,
            });
            Status::Failed
        } else if !metric_errors.is_empty() {
            Status::Failed
        } else {
            Status::Ok
        };
        problems.extend(metric_errors);
        if row.status != Status::Ok {
            let stderr = tail(&exec.stderr, 1500);
            if !stderr.is_empty() {
                problems.push(format!("standard error ends with: {stderr}"));
            }
            row.error = Some(problems.join("; "));
        }
        Ok(row)
    }

    fn plan_fields(&self, row: &mut Row, plan: &super::spec::PlanDef, stdout: &str) {
        let pointer = plan.estimate.as_deref().unwrap_or("/estimate");
        let estimate = last_json_object(stdout)
            .and_then(|doc| doc.pointer(pointer).cloned())
            .and_then(|v| parse_estimate(&v).ok());
        let mut actual = BTreeMap::new();
        for q in PLAN_QUANTITIES {
            let source = plan.actual.get(q).map_or(
                match q {
                    "seconds" => "wall_seconds",
                    "peak_memory_bytes" => "memory_peak_bytes",
                    other => other,
                },
                String::as_str,
            );
            if let Some(v) = row.metric(source) {
                actual.insert(q.to_string(), v);
            }
        }
        row.estimate_error = estimate.as_ref().map(|e| estimate_errors(e, &actual));
        row.estimate = estimate;
        row.actual = Some(actual);
    }

    fn progress(&mut self, n: usize, total: usize, row: &Row) {
        let mut line = format!(
            "[{n}/{total}] {} rep {}/{}",
            row.case, row.repetition, row.repetitions
        );
        if let Some(v) = row.metric(&row.primary) {
            line.push_str(&format!(
                "  {} {}",
                row.primary,
                super::report::format_value(v, row.unit(&row.primary))
            ));
        }
        if let Some(t) = row.time {
            line.push_str(&format!("  wall {:.3} s", t.wall_seconds));
        }
        if let Some(m) = &row.memory {
            line.push_str(&format!(
                "  peak {} (anon {})",
                human_size(m.peak_bytes),
                human_size(m.anon_peak_bytes)
            ));
        }
        line.push_str(&format!("  {}", row.status.as_str()));
        if row.disturbance.disturbed {
            line.push_str(&format!(
                "  DISTURBED ({})",
                row.disturbance.reasons.join("; ")
            ));
        }
        if let Some(e) = &row.error {
            line.push_str(&format!("\n    {}", tail(e, 600)));
        }
        say!(self.log, "{line}");
    }
}

/// The file that marks a directory as a work directory of the harness, so that `clean`
/// never deletes anything else.
pub const WORK_MARKER: &str = ".everygraph-bench-work";

fn mark_work_dir(work: &Path) -> Result<(), String> {
    let marker = work.join(WORK_MARKER);
    if !marker.exists() {
        std::fs::write(
            &marker,
            "Work directory of everygraph-bench; `everygraph-bench clean` deletes it.\n",
        )
        .map_err(|e| format!("writing {}: {e}", marker.display()))?;
    }
    Ok(())
}

/// Runs a spec.
///
/// # Errors
///
/// [`RunError::Refused`] before anything ran (lock, gate, dirty tree);
/// [`RunError::Failed`] when the harness could not go on.
pub fn run(
    spec: &Spec,
    ctx: &Context,
    opts: &RunOptions,
    log: &mut dyn Write,
) -> Result<RunOutcome, RunError> {
    let cases = spec.select(&opts.filters);
    if cases.is_empty() {
        return Err(RunError::Failed(format!(
            "no case of {} matches {:?}; everygraph-bench check {} lists them",
            spec.name, opts.filters, spec.name
        )));
    }
    let git = GitInfo::read(&ctx.repo)?;
    let started = unix_now();
    let run_id = format!(
        "{}-{}{}",
        compact_timestamp(started),
        git.short(),
        if git.dirty { "-dirty" } else { "" }
    );

    // The one lock, taken here and nowhere else, before any other check: a second
    // official run learns first that one is in progress.
    let _lock = if opts.official {
        let holder = format!(
            "process {}, spec {}, run {run_id}, started {}",
            std::process::id(),
            spec.name,
            rfc3339(started)
        );
        match OfficialLock::take(&opts.lock_path, &holder) {
            Ok(l) => Some(l),
            Err(e @ LockError::Held { .. }) => return Err(RunError::Refused(e.to_string())),
            Err(e) => return Err(RunError::Failed(e.to_string())),
        }
    } else {
        None
    };
    if opts.official && git.dirty && !opts.allow_dirty {
        return Err(RunError::Refused(
            "the working tree has uncommitted changes to tracked files, so the revision \
             would not reproduce an official result: commit first, or pass --allow-dirty \
             (every row then says dirty)"
                .into(),
        ));
    }
    let spec_path = spec
        .path
        .as_deref()
        .map(|p| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()));
    let spec_rel = spec_path
        .as_deref()
        .and_then(|p| p.strip_prefix(&ctx.repo).ok())
        .map(|p| p.display().to_string());
    if opts.official {
        match (&spec_path, &spec_rel) {
            (Some(_), Some(rel)) => {
                GitInfo::file_at_head(&ctx.repo, rel).map_err(RunError::Refused)?;
            }
            _ => {
                return Err(RunError::Refused(format!(
                    "an official run reads its spec from the repository, committed: {} is \
                     outside {}",
                    spec_path
                        .as_deref()
                        .map_or_else(|| "this spec".into(), |p| p.display().to_string()),
                    ctx.repo.display()
                )));
            }
        }
        if let Some(m) = opts.max_load.filter(|m| *m > spec.gate.max_load) {
            return Err(RunError::Refused(format!(
                "--max-load {m} is above the spec's {}: an official run may make the gate \
                 stricter, not looser (change [gate] max_load in the spec, committed, if \
                 the threshold is wrong)",
                spec.gate.max_load
            )));
        }
    }
    let mut sources: BTreeMap<String, PathBuf> = BTreeMap::new();
    for c in &cases {
        if let (Some(d), None) = (&c.dataset, &c.unavailable) {
            let p = dataset_path(ctx, d)?;
            if !p.exists() {
                return Err(RunError::Failed(format!(
                    "case {}: dataset {} is not at {}; fetch it (everygraph-bench fetch) or \
                     point --data / $EVERYGRAPH_DATA at the data root",
                    c.id,
                    d.label(),
                    p.display()
                )));
            }
            sources.insert(c.id.clone(), p);
        }
    }
    let active: Vec<&&Case> = cases.iter().filter(|c| c.unavailable.is_none()).collect();
    std::fs::create_dir_all(&ctx.work)
        .map_err(|e| format!("creating the work directory {}: {e}", ctx.work.display()))?;
    refuse_memory_backed(
        "the work directory",
        &ctx.work,
        "Point --work or $EVERYGRAPH_BENCH_WORK at a directory on disk (or set \
         $EVERYGRAPH_SCRATCH, whose bench-work subdirectory is the default).",
    )?;
    mark_work_dir(&ctx.work)?;
    for c in active.iter().filter(|c| c.cache == CacheMode::Cold) {
        refuse_memory_backed(
            &format!("the dataset of {}", c.id),
            &sources[&c.id],
            "Cold-cache cases need their data on disk.",
        )?;
    }
    let mut gate_record = None;
    if opts.official {
        let readings = StartReadings {
            load: ctx.host.loadavg()?,
            mem_available: ctx.host.mem_available()?,
        };
        let need = active.iter().map(|c| c.memory_limit).max().unwrap_or(0);
        let max_load = opts.max_load.unwrap_or(spec.gate.max_load);
        gate::check_start(&readings, &spec.gate, need, max_load).map_err(RunError::Refused)?;
        say!(
            log,
            "idle gate passed: load {:.2} (limit {max_load:.2}), {} available",
            readings.load.one,
            human_size(readings.mem_available)
        );
        gate_record = Some(GateRecord {
            lock_path: opts.lock_path.display().to_string(),
            max_load,
            spec_max_load: spec.gate.max_load,
            min_mem_available_bytes: spec.gate.min_mem_available.unwrap_or(need),
            disturbed_above_cores: spec.gate.disturbed_above_cores,
            disturbed_above_io_mb_s: spec.gate.disturbed_above_io_mb_s,
            readings,
        });
    }
    let any_cold = active.iter().any(|c| c.cache == CacheMode::Cold);
    preflight(any_cold)?;

    let mut runner = Runner {
        spec,
        ctx,
        opts,
        machine: Fingerprint::read(&ctx.host)?,
        git,
        run_id,
        log,
        copies: BTreeMap::new(),
        gate: gate_record,
        spec_path: spec_rel,
        hashes: BTreeMap::new(),
        listings: BTreeMap::new(),
    };
    say!(
        runner.log,
        "{} run {} of {}: {} cases; machine: {}",
        if opts.official {
            "official"
        } else {
            "exploratory"
        },
        runner.run_id,
        spec.name,
        cases.len(),
        runner.machine.summary()
    );
    if !opts.skip_prepare {
        let selected: Vec<&Case> = active.iter().map(|c| **c).collect();
        for step in spec.prepare.iter().filter(|s| s.needed(&selected)) {
            runner.prepare(step)?;
        }
    }
    for c in active.iter().filter(|c| c.cache == CacheMode::Cold) {
        let src = &sources[&c.id];
        if runner.copies.contains_key(src) {
            continue;
        }
        let n = active
            .iter()
            .filter(|o| o.cache == CacheMode::Cold && sources.get(&o.id) == Some(src))
            .map(|o| o.cold_copies)
            .max()
            .unwrap_or(2);
        // `<name>-<hash of the full path>`: two datasets of the same name stay apart.
        let digest = crate::datasets::hex(&<sha2::Sha256 as sha2::Digest>::digest(
            src.display().to_string().as_bytes(),
        ));
        let name = src
            .file_name()
            .map_or_else(|| "data".into(), |n| path_safe(&n.to_string_lossy()));
        let dir = ctx
            .work
            .join("copies")
            .join(format!("{name}-{}", &digest[..12]));
        let log_ref = &mut *runner.log;
        let copies = cache::prepare_copies(src, &dir, n, &mut |l| say!(log_ref, "{l}"))?;
        runner.copies.insert(src.clone(), copies);
    }
    if opts.prepare_only {
        return Ok(RunOutcome::default());
    }

    let results = opts.out.clone().unwrap_or_else(|| {
        let dir = if opts.official {
            "results"
        } else {
            "exploratory"
        };
        ctx.repo
            .join("bench")
            .join(dir)
            .join(&spec.name)
            .join(format!("{}.jsonl", runner.run_id))
    });
    let mut writer = RowWriter::create(&results)?;
    let mut outcome = RunOutcome {
        results: Some(results.clone()),
        ..RunOutcome::default()
    };
    let mut disturbed = Vec::new();
    let total: u32 = cases
        .iter()
        .map(|c| {
            if c.unavailable.is_some() {
                1
            } else {
                opts.repetitions.unwrap_or(c.repetitions)
            }
        })
        .sum();
    let total = total as usize;
    let mut n = 0;
    for case in &cases {
        let reps = opts.repetitions.unwrap_or(case.repetitions);
        if let Some(reason) = &case.unavailable {
            let mut row = runner.base_row(case, 0, reps);
            row.status = Status::Unavailable;
            row.error = Some(reason.clone());
            writer.write(&row)?;
            n += 1;
            outcome.rows += 1;
            outcome.unavailable += 1;
            say!(
                runner.log,
                "[{n}/{total}] {}: unavailable: {reason}",
                case.id
            );
            continue;
        }
        let source = sources.get(&case.id).map(PathBuf::as_path);
        let mut warm_failed = None;
        for w in 1..=case.warmup {
            let exec = runner.execute(case, &format!("warmup{w}"), source)?;
            let ok = exec.error.is_none()
                && exec
                    .measurement
                    .as_ref()
                    .is_some_and(|m| m.exit_code == Some(0) && !m.timed_out);
            if !ok {
                warm_failed = Some(format!(
                    "the warm-up run failed ({}); standard error ends with: {}",
                    exec.error.unwrap_or_else(|| "non-zero exit".into()),
                    tail(&exec.stderr, 1200)
                ));
                break;
            }
        }
        if let Some(reason) = warm_failed {
            let mut row = runner.base_row(case, 0, reps);
            row.status = Status::Failed;
            row.error = Some(reason);
            writer.write(&row)?;
            n += reps as usize;
            outcome.rows += 1;
            outcome.failed += 1;
            say!(
                runner.log,
                "[{n}/{total}] {}: {}",
                case.id,
                row.error.as_deref().unwrap_or("")
            );
            continue;
        }
        for rep in 1..=reps {
            let row = runner.repetition(case, source, rep, reps)?;
            writer.write(&row)?;
            n += 1;
            outcome.rows += 1;
            if row.status != Status::Ok {
                outcome.failed += 1;
            }
            if row.disturbance.disturbed {
                outcome.disturbed += 1;
                disturbed.push(format!("{} rep {}", row.case, row.repetition));
            }
            runner.progress(n, total, &row);
        }
    }
    say!(
        runner.log,
        "results: {} ({} rows)",
        results.display(),
        outcome.rows
    );
    if !disturbed.is_empty() {
        say!(
            runner.log,
            "{} row(s) were DISTURBED by other activity and are excluded from summaries: {}",
            disturbed.len(),
            disturbed.join(", ")
        );
    }
    if outcome.failed > 0 {
        say!(
            runner.log,
            "{} row(s) did not complete; see their error fields",
            outcome.failed
        );
    }
    Ok(outcome)
}
