// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One result row: one repetition of one case, as a line of JSON. Rows are
//! self-contained — machine, revision, command and conditions are in each — so that a
//! row copied out of its file still says what it measured. The schema is documented in
//! `bench/README.md`. A change that rows of the current version cannot be read under, or
//! that changes what an existing field means, bumps [`ROW_SCHEMA`]; a new field that
//! older rows lack is read with a default that states what those rows meant, and keeps
//! the version (so that committed results stay readable).

use std::collections::BTreeMap;
use std::io::{BufRead as _, Write as _};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::gate::Disturbance;
use super::machine::{Fingerprint, GitInfo};
use super::measure::StopRecord;
use super::procfs::LoadAvg;
use super::spec::{CacheMode, PLAN_QUANTITIES};

/// The row format version.
pub const ROW_SCHEMA: u32 = 1;

/// How a repetition ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Exit status 0 and every required metric read.
    Ok,
    /// Non-zero exit, a signal, or a required metric missing.
    Failed,
    /// Killed at the spec's timeout.
    Timeout,
    /// The kernel's OOM killer ended a process of the run.
    Oom,
    /// The cold-cache check found the dataset still in the page cache; not measured.
    NotCold,
    /// The spec marks the benchmark unavailable on this machine; not run.
    Unavailable,
    /// The harness itself failed around the run (see `error`).
    HarnessError,
    /// The command exited but left processes in its scope; they were killed.
    LeftProcesses,
}

impl Status {
    /// The word used in reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Failed => "failed",
            Status::Timeout => "timeout",
            Status::Oom => "oom",
            Status::NotCold => "not_cold",
            Status::Unavailable => "unavailable",
            Status::HarnessError => "harness_error",
            Status::LeftProcesses => "left_processes",
        }
    }
}

/// The data a row ran on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetRecord {
    /// Registry name or the path as the spec wrote it.
    pub name: String,
    /// Where it was read from (a private copy for cold cache).
    pub path: String,
    /// Total size of its files.
    pub bytes: u64,
    /// SHA-256 of its listing (relative path, size and modification time of each file):
    /// equal listings mean the same files, without reading gigabytes.
    pub listing_sha256: String,
}

/// The idle gate of an official run, as applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateRecord {
    /// The lock file taken.
    pub lock_path: String,
    /// The load threshold applied (the spec's, or a lower `--max-load`).
    pub max_load: f64,
    /// The spec's own load threshold.
    pub spec_max_load: f64,
    /// The `MemAvailable` the run needed.
    pub min_mem_available_bytes: u64,
    /// Thresholds of the disturbance check.
    pub disturbed_above_cores: f64,
    /// MB/s of others' storage traffic.
    pub disturbed_above_io_mb_s: f64,
    /// What the gate read at the start of the run.
    pub readings: super::gate::StartReadings,
}

/// The page-cache state a row started from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheRecord {
    /// Warm or cold.
    pub mode: CacheMode,
    /// For cold cache: which private copy (they rotate).
    pub copy: Option<u32>,
    /// For cold cache: bytes of the copy still in the page cache after eviction, as
    /// `mincore(2)` reports it through util-linux `fincore`.
    pub resident_bytes: Option<u64>,
}

/// Times measured by the harness.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeRecord {
    /// Start to exit of the command, monotonic clock.
    pub wall_seconds: f64,
    /// User CPU of everything in the run's cgroup.
    pub user_seconds: f64,
    /// System CPU of everything in the run's cgroup.
    pub system_seconds: f64,
}

/// Storage traffic of the command and its descendants, orphaned ones included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IoRecord {
    /// From storage (page-cache hits not counted).
    pub read_bytes: u64,
    /// To storage.
    pub write_bytes: u64,
    /// Why the run's own traffic could not be established, if it could not (a
    /// descendant could not be reaped). The two counts are then a lower bound, the
    /// `read_bytes`/`write_bytes` metrics are missing, and the disturbance check does not
    /// compute others' traffic during the run. Absent in rows written before it existed.
    #[serde(default)]
    pub unknown: Option<String>,
}

/// Memory of the run's cgroup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    /// `memory.max` as the kernel applied it.
    pub limit_bytes: Option<u64>,
    /// `memory.swap.max`.
    pub swap_limit_bytes: Option<u64>,
    /// `memory.peak`: total of anonymous, file-backed and kernel memory, exact.
    pub peak_bytes: u64,
    /// Largest anonymous memory sampled. This is what a planner must budget.
    pub anon_peak_bytes: u64,
    /// Largest page cache charged to the run, sampled. Memory maps live here, which
    /// is why RSS alone misleads (FINDINGS § B1).
    pub file_peak_bytes: u64,
    /// Number of samples.
    pub samples: u64,
    /// Sampling interval.
    pub sample_interval_ms: f64,
}

/// Memory-limit events of the run's cgroup (`memory.events`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OomRecord {
    /// Times usage hit the limit and reclaim had to run (`max`).
    pub max_events: u64,
    /// Times reclaim failed (`oom`).
    pub oom_events: u64,
    /// Processes killed by the OOM killer (`oom_kill`).
    pub oom_kills: u64,
}

/// How the command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitRecord {
    /// Exit code, if it exited.
    pub code: Option<i32>,
    /// Signal, if one ended it.
    pub signal: Option<i32>,
    /// Killed at the timeout.
    pub timed_out: bool,
}

/// One estimated quantity (ux-contract P-7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EstimateValue {
    /// Point estimate.
    pub value: f64,
    /// Lower end of the interval.
    pub low: f64,
    /// Upper end.
    pub high: f64,
    /// `exact`, `sampled` or `model`.
    pub basis: String,
}

/// The error of one estimate against its actual (ux-contract P-9).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct QuantityError {
    /// `(actual − value) / actual`; `0` when both are 0; `None` (undefined) when only
    /// the actual is 0.
    pub relative: Option<f64>,
    /// `value − actual`, in the quantity's unit.
    pub absolute: f64,
    /// `low ≤ actual ≤ high`.
    pub inside: bool,
}

/// P-9 errors for every quantity that has both an estimate and an actual.
#[must_use]
pub fn estimate_errors(
    estimate: &BTreeMap<String, EstimateValue>,
    actual: &BTreeMap<String, f64>,
) -> BTreeMap<String, QuantityError> {
    estimate
        .iter()
        .filter_map(|(q, e)| {
            let a = *actual.get(q)?;
            let relative = if a != 0.0 {
                Some((a - e.value) / a)
            } else if e.value == 0.0 {
                Some(0.0)
            } else {
                None
            };
            Some((
                q.clone(),
                QuantityError {
                    relative,
                    absolute: e.value - a,
                    inside: e.low <= a && a <= e.high,
                },
            ))
        })
        .collect()
}

/// Reads an `estimate` object: the quantities of [`PLAN_QUANTITIES`] that are present.
///
/// # Errors
///
/// The value is not an object of `{value, low, high, basis}` entries.
pub fn parse_estimate(v: &serde_json::Value) -> Result<BTreeMap<String, EstimateValue>, String> {
    let obj = v.as_object().ok_or("the estimate is not a JSON object")?;
    let mut out = BTreeMap::new();
    for q in PLAN_QUANTITIES {
        if let Some(e) = obj.get(q) {
            let e: EstimateValue =
                serde_json::from_value(e.clone()).map_err(|err| format!("estimate.{q}: {err}"))?;
            out.insert(q.to_string(), e);
        }
    }
    Ok(out)
}

/// A published value as the row recorded it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceRecord {
    /// The metric compared.
    pub metric: String,
    /// The value, or
    pub value: Option<f64>,
    /// the bound.
    pub max: Option<f64>,
    /// Relative tolerance for `value`.
    pub tolerance: f64,
    /// Where it is published.
    pub source: String,
}

/// One repetition of one case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// [`ROW_SCHEMA`].
    pub schema: u32,
    /// `<timestamp>-<revision>`, shared by the rows of one run.
    pub run_id: String,
    /// The spec's name.
    pub spec: String,
    /// SHA-256 of the spec file.
    pub spec_sha256: String,
    /// The spec file, relative to the repository when inside it.
    pub spec_path: Option<String>,
    /// Official (lock, idle gate) or exploratory.
    pub official: bool,
    /// Case id.
    pub case: String,
    /// Benchmark name.
    pub benchmark: String,
    /// Report table.
    pub group: String,
    /// Matrix values.
    pub vars: BTreeMap<String, String>,
    /// Comparison runner, if any.
    pub runner: Option<String>,
    /// 1-based repetition number.
    pub repetition: u32,
    /// Repetitions of the case.
    pub repetitions: u32,
    /// For the row of a warm-up run that failed (`repetition` 0, never counted): which
    /// warm-up it was, 1-based. `None` for every other row, and in rows written before
    /// the field existed.
    #[serde(default)]
    pub warmup: Option<u32>,
    /// The seed of an interleaved run (`run --interleave`); `None` when the cases ran
    /// one after another.
    #[serde(default)]
    pub seed: Option<u64>,
    /// The 1-based round of an interleaved run this repetition ran in.
    #[serde(default)]
    pub round: Option<u32>,
    /// The 1-based place of the case in its round.
    #[serde(default)]
    pub position: Option<u32>,
    /// How it ended.
    pub status: Status,
    /// Why, when not `ok`.
    pub error: Option<String>,
    /// UTC start time.
    pub started_at: String,
    /// The machine.
    pub machine: Fingerprint,
    /// The revision.
    pub git: GitInfo,
    /// The measured command, fully expanded.
    pub command: Vec<String>,
    /// SHA-256 of each file the spec declares the commands run (`binaries`), by path.
    pub binaries: BTreeMap<String, String>,
    /// The idle gate of an official run.
    pub gate: Option<GateRecord>,
    /// The `systemd-run` wrapper around it.
    pub wrapper: Vec<String>,
    /// Working directory.
    pub cwd: Option<String>,
    /// Extra environment.
    pub env: BTreeMap<String, String>,
    /// The data.
    pub dataset: Option<DatasetRecord>,
    /// Conditions the report prints with every number (graph, precision, …).
    pub conditions: BTreeMap<String, String>,
    /// Thread count.
    pub threads: u32,
    /// The hard memory limit asked for.
    pub memory_limit_bytes: u64,
    /// Page-cache state.
    pub cache: CacheRecord,
    /// Load averages before the repetition.
    pub load_start: Option<LoadAvg>,
    /// Load averages after it.
    pub load_end: Option<LoadAvg>,
    /// `MemAvailable` before it.
    pub mem_available_start_bytes: Option<u64>,
    /// Activity of other processes.
    pub disturbance: Disturbance,
    /// Times.
    pub time: Option<TimeRecord>,
    /// Storage traffic.
    pub io: Option<IoRecord>,
    /// Memory.
    pub memory: Option<MemoryRecord>,
    /// Memory-limit events.
    pub oom: Option<OomRecord>,
    /// Exit.
    pub exit: Option<ExitRecord>,
    /// The interrupt and how long the stop took (C-4).
    pub stop: Option<StopRecord>,
    /// Processes the command left in its scope (killed; the row is not ok).
    pub left_processes: Vec<u32>,
    /// Text fields that say what ran (`tool`, `version`, `algorithm`, `options` for a
    /// comparison runner), from the spec's `record` and `record_json`.
    pub recorded: BTreeMap<String, String>,
    /// Metrics read from the output.
    pub metrics: BTreeMap<String, f64>,
    /// Units of the metrics, where the spec gives one.
    pub units: BTreeMap<String, String>,
    /// The metric the report leads with.
    pub primary: String,
    /// Published values to compare with.
    pub references: Vec<ReferenceRecord>,
    /// The command's estimates (P-7), when its spec has a `plan`.
    pub estimate: Option<BTreeMap<String, EstimateValue>>,
    /// The actuals of the four quantities (P-9).
    pub actual: Option<BTreeMap<String, f64>>,
    /// Their errors (P-9).
    pub estimate_error: Option<BTreeMap<String, QuantityError>>,
}

impl Row {
    /// A metric by name: one read from the output, or one the harness measured
    /// ([`super::spec::BUILTIN_METRICS`]).
    #[must_use]
    pub fn metric(&self, name: &str) -> Option<f64> {
        #[allow(
            clippy::cast_precision_loss,
            reason = "byte counts shown to a few digits"
        )]
        let b = |x: u64| x as f64;
        match name {
            "wall_seconds" => self.time.map(|t| t.wall_seconds),
            "user_seconds" => self.time.map(|t| t.user_seconds),
            "system_seconds" => self.time.map(|t| t.system_seconds),
            "memory_peak_bytes" => self.memory.as_ref().map(|m| b(m.peak_bytes)),
            "anon_peak_bytes" => self.memory.as_ref().map(|m| b(m.anon_peak_bytes)),
            "file_peak_bytes" => self.memory.as_ref().map(|m| b(m.file_peak_bytes)),
            "read_bytes" => self.known_io().map(|i| b(i.read_bytes)),
            "write_bytes" => self.known_io().map(|i| b(i.write_bytes)),
            "stop_seconds" => self.stop.as_ref().and_then(|s| s.stop_seconds),
            other => self.metrics.get(other).copied(),
        }
    }

    /// The storage traffic, when the run's own is known.
    fn known_io(&self) -> Option<&IoRecord> {
        self.io.as_ref().filter(|i| i.unknown.is_none())
    }

    /// The unit of a metric.
    #[must_use]
    pub fn unit(&self, name: &str) -> Option<&str> {
        super::spec::BUILTIN_METRICS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, u)| *u)
            .or_else(|| self.units.get(name).map(String::as_str))
    }

    /// Counted in summaries: completed, not disturbed.
    #[must_use]
    pub fn counts(&self) -> bool {
        self.status == Status::Ok && !self.disturbance.disturbed
    }
}

/// Appends rows to a JSON-lines file, one flushed line each, so that an interrupted
/// run keeps what it measured.
#[derive(Debug)]
pub struct RowWriter {
    file: std::fs::File,
}

impl RowWriter {
    /// Creates `path` (and its directory); it must not exist.
    ///
    /// # Errors
    ///
    /// It exists or cannot be created.
    pub fn create(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| format!("creating {}: {e}", path.display()))?;
        Ok(Self { file })
    }

    /// Writes one row.
    ///
    /// # Errors
    ///
    /// The write fails.
    pub fn write(&mut self, row: &Row) -> Result<(), String> {
        let line = serde_json::to_string(row).map_err(|e| e.to_string())?;
        writeln!(self.file, "{line}")
            .and_then(|()| self.file.flush())
            .map_err(|e| format!("writing a result row: {e}"))
    }
}

/// Reads the rows of a JSON-lines file.
///
/// # Errors
///
/// The file cannot be read, or a line is not a row of this schema (the message names
/// the line).
pub fn read_rows(path: &Path) -> Result<Vec<Row>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("opening {}: {e}", path.display()))?;
    let mut rows = Vec::new();
    for (i, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| format!("reading {}: {e}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line)
            .map_err(|e| format!("{}:{}: not JSON: {e}", path.display(), i + 1))?;
        let schema = value.get("schema").and_then(serde_json::Value::as_u64);
        if schema != Some(u64::from(ROW_SCHEMA)) {
            return Err(format!(
                "{}:{}: row schema {schema:?}, but this everygraph-bench reads {ROW_SCHEMA}",
                path.display(),
                i + 1
            ));
        }
        rows.push(
            serde_json::from_value(value)
                .map_err(|e| format!("{}:{}: not a result row: {e}", path.display(), i + 1))?,
        );
    }
    Ok(rows)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::datasets::test_support::TempDir;
    use crate::harness::gate::{Activity, Disturbance, classify};
    use crate::harness::spec::Gate;

    /// The gate of the tests: 0.5 cores, 50 MB/s.
    pub(crate) fn test_gate() -> Gate {
        Gate {
            max_load: 1.0,
            disturbed_above_cores: 0.5,
            disturbed_above_io_mb_s: 50.0,
            min_mem_available: None,
        }
    }

    /// An undisturbed reading.
    pub(crate) fn quiet() -> Disturbance {
        classify(
            Activity {
                settle_busy_cores: Some(0.05),
                ..Activity::default()
            },
            &test_gate(),
            true,
        )
    }

    /// A disturbed one (official run).
    pub(crate) fn busy() -> Disturbance {
        classify(
            Activity {
                settle_busy_cores: Some(1.5),
                ..Activity::default()
            },
            &test_gate(),
            true,
        )
    }

    /// A complete row with made-up numbers, for the tests of this module and the report.
    pub(crate) fn sample_row(case: &str, rep: u32, primary_value: f64) -> Row {
        Row {
            schema: ROW_SCHEMA,
            run_id: "20261125T090642Z-5e4aad8d2f1c".into(),
            spec: "demo".into(),
            spec_sha256: "0".repeat(64),
            spec_path: Some("bench/specs/demo.toml".into()),
            official: true,
            case: case.into(),
            benchmark: case.split('/').next().unwrap_or(case).into(),
            group: "kernels".into(),
            vars: BTreeMap::new(),
            runner: None,
            repetition: rep,
            repetitions: 3,
            warmup: None,
            seed: None,
            round: None,
            position: None,
            status: Status::Ok,
            error: None,
            started_at: "2026-11-25T09:06:42Z".into(),
            machine: Fingerprint {
                id: "0123456789abcdef".into(),
                cpu_model: "Test CPU".into(),
                cores: 16,
                threads: 32,
                smt: Some("on".into()),
                memory_bytes: 64 << 30,
                kernel: "6.12.0".into(),
                governors: vec!["performance".into()],
                boost: Some(true),
                thp: Some("madvise".into()),
            },
            git: GitInfo {
                revision: "5e4aad8d2f1c0000000000000000000000000000".into(),
                dirty: false,
            },
            command: vec!["glab".into(), "run".into()],
            binaries: [("lab/glab".to_string(), "1".repeat(64))]
                .into_iter()
                .collect(),
            gate: Some(GateRecord {
                lock_path: "/run/user/1000/everygraph-bench-official.lock".into(),
                max_load: 1.0,
                spec_max_load: 1.0,
                min_mem_available_bytes: 8 << 30,
                disturbed_above_cores: 0.5,
                disturbed_above_io_mb_s: 50.0,
                readings: crate::harness::gate::StartReadings {
                    load: LoadAvg {
                        one: 0.1,
                        five: 0.1,
                        fifteen: 0.1,
                    },
                    mem_available: 60 << 30,
                },
            }),
            wrapper: vec!["systemd-run".into()],
            cwd: None,
            env: BTreeMap::new(),
            dataset: Some(DatasetRecord {
                name: "csr/orkut".into(),
                path: "/data/csr/orkut".into(),
                bytes: 1 << 30,
                listing_sha256: "2".repeat(64),
            }),
            conditions: [("graph", "orkut"), ("precision", "f32")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            threads: 32,
            memory_limit_bytes: 8 << 30,
            cache: CacheRecord {
                mode: CacheMode::Warm,
                copy: None,
                resident_bytes: None,
            },
            load_start: Some(LoadAvg {
                one: 0.1,
                five: 0.1,
                fifteen: 0.1,
            }),
            load_end: Some(LoadAvg {
                one: 3.0,
                five: 0.5,
                fifteen: 0.2,
            }),
            mem_available_start_bytes: Some(60 << 30),
            disturbance: quiet(),
            time: Some(TimeRecord {
                wall_seconds: primary_value + 0.5,
                user_seconds: 10.0,
                system_seconds: 1.0,
            }),
            io: Some(IoRecord {
                read_bytes: 0,
                write_bytes: 4096,
                unknown: None,
            }),
            memory: Some(MemoryRecord {
                limit_bytes: Some(8 << 30),
                swap_limit_bytes: Some(0),
                peak_bytes: 1 << 30,
                anon_peak_bytes: 900 << 20,
                file_peak_bytes: 100 << 20,
                samples: 40,
                sample_interval_ms: 20.0,
            }),
            oom: Some(OomRecord {
                max_events: 0,
                oom_events: 0,
                oom_kills: 0,
            }),
            exit: Some(ExitRecord {
                code: Some(0),
                signal: None,
                timed_out: false,
            }),
            stop: None,
            left_processes: vec![],
            recorded: BTreeMap::new(),
            metrics: [("seconds".to_string(), primary_value)]
                .into_iter()
                .collect(),
            units: [("seconds".to_string(), "s".to_string())]
                .into_iter()
                .collect(),
            primary: "seconds".into(),
            references: vec![],
            estimate: None,
            actual: None,
            estimate_error: None,
        }
    }

    #[test]
    fn rows_round_trip_through_a_file() {
        let dir = TempDir::new("rows");
        let path = dir.path().join("r").join("run.jsonl");
        let mut w = RowWriter::create(&path).expect("new file");
        let rows = [sample_row("pr32", 1, 0.38), sample_row("pr32", 2, 0.39)];
        for r in &rows {
            w.write(r).expect("write");
        }
        assert!(
            RowWriter::create(&path).is_err(),
            "never overwrites a result file"
        );
        assert_eq!(read_rows(&path).expect("read"), rows);
    }

    /// The fields added by #142 and #195 are additive: a row written before them reads
    /// with "not a warm-up", "not interleaved" and "I/O known", and keeps its metrics.
    #[test]
    fn rows_without_the_warmup_and_io_unknown_fields_still_read() {
        let mut v = serde_json::to_value(sample_row("pr32", 1, 0.38)).expect("serializes");
        let obj = v.as_object_mut().expect("object");
        for k in ["warmup", "seed", "round", "position"] {
            obj.remove(k);
        }
        obj["io"].as_object_mut().expect("io").remove("unknown");
        let dir = TempDir::new("old-rows");
        let path = dir.path().join("old.jsonl");
        std::fs::write(&path, format!("{v}\n")).expect("write");
        let rows = read_rows(&path).expect("an older row reads");
        assert_eq!(rows[0], sample_row("pr32", 1, 0.38));
        assert_eq!(rows[0].metric("write_bytes"), Some(4096.0));
    }

    #[test]
    fn unknown_io_is_no_metric() {
        let mut r = sample_row("pr32", 1, 0.38);
        r.io = Some(IoRecord {
            read_bytes: 86_016,
            write_bytes: 0,
            unknown: Some("a descendant could not be reaped".into()),
        });
        assert_eq!(r.metric("read_bytes"), None);
        assert_eq!(r.metric("write_bytes"), None);
    }

    #[test]
    fn the_row_schema_has_the_fields_the_issue_asks_for() {
        let v = serde_json::to_value(sample_row("pr32", 1, 0.38)).expect("serializes");
        for pointer in [
            "/machine/cpu_model",
            "/machine/cores",
            "/machine/threads",
            "/machine/memory_bytes",
            "/machine/kernel",
            "/machine/governors",
            "/machine/smt",
            "/git/revision",
            "/git/dirty",
            "/command",
            "/threads",
            "/memory_limit_bytes",
            "/load_start/one",
            "/load_end/one",
            "/mem_available_start_bytes",
            "/time/wall_seconds",
            "/time/user_seconds",
            "/time/system_seconds",
            "/io/read_bytes",
            "/io/write_bytes",
            "/io/unknown",
            "/warmup",
            "/memory/peak_bytes",
            "/memory/anon_peak_bytes",
            "/memory/file_peak_bytes",
            "/oom/oom_kills",
            "/exit/code",
            "/metrics/seconds",
            "/disturbance/disturbed",
            "/disturbance/settle_io_mb_s",
            "/gate/lock_path",
            "/gate/max_load",
            "/gate/readings/load/one",
            "/binaries",
            "/recorded",
            "/left_processes",
            "/dataset/listing_sha256",
            "/spec_path",
            "/estimate",
            "/actual",
            "/estimate_error",
        ] {
            assert!(v.pointer(pointer).is_some(), "row has no {pointer}");
        }
    }

    #[test]
    fn other_schemas_and_garbage_are_refused_with_the_line() {
        let dir = TempDir::new("rows-bad");
        let path = dir.path().join("bad.jsonl");
        let good = serde_json::to_string(&sample_row("a", 1, 1.0)).expect("json");
        std::fs::write(
            &path,
            format!("{good}\n{}\n", good.replace("\"schema\":1", "\"schema\":9")),
        )
        .expect("write");
        assert!(
            read_rows(&path)
                .unwrap_err()
                .contains("bad.jsonl:2: row schema Some(9)")
        );
        std::fs::write(&path, "not json\n").expect("write");
        assert!(read_rows(&path).unwrap_err().contains(":1: not JSON"));
    }

    #[test]
    fn p9_errors_including_a_zero_actual() {
        let est = |value: f64, low: f64, high: f64| EstimateValue {
            value,
            low,
            high,
            basis: "model".into(),
        };
        let estimate: BTreeMap<String, EstimateValue> = [
            ("seconds".to_string(), est(100.0, 80.0, 120.0)),
            ("peak_memory_bytes".to_string(), est(5.0, 4.0, 6.0)),
            ("scratch_bytes".to_string(), est(0.0, 0.0, 0.0)),
            ("output_bytes".to_string(), est(10.0, 5.0, 15.0)),
        ]
        .into_iter()
        .collect();
        let actual: BTreeMap<String, f64> = [
            ("seconds".to_string(), 125.0),
            ("peak_memory_bytes".to_string(), 5.0),
            ("scratch_bytes".to_string(), 0.0),
            ("output_bytes".to_string(), 0.0),
        ]
        .into_iter()
        .collect();
        let e = estimate_errors(&estimate, &actual);
        assert_eq!(e["seconds"].relative, Some(0.2));
        assert_eq!(e["seconds"].absolute, -25.0);
        assert!(!e["seconds"].inside);
        assert!(e["peak_memory_bytes"].inside);
        assert_eq!(
            e["scratch_bytes"].relative,
            Some(0.0),
            "0 for 0 is no error"
        );
        assert_eq!(
            e["output_bytes"].relative, None,
            "undefined when only the actual is 0"
        );
        assert_eq!(e["output_bytes"].absolute, 10.0);

        let parsed = parse_estimate(&serde_json::json!({
            "seconds": {"value": 220, "low": 160, "high": 280, "basis": "sampled"},
            "unrelated": 1
        }))
        .expect("valid");
        assert_eq!(parsed["seconds"].high, 280.0);
        assert!(parse_estimate(&serde_json::json!({"seconds": {"value": 1}})).is_err());
    }
}
