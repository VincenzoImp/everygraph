// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Benchmark specs: the TOML files under `bench/specs/` (format in `bench/README.md`).
//!
//! A spec names its benchmarks; each benchmark is a command template run on a dataset
//! under a hard memory limit, expanded over a thread list and an optional matrix of
//! variables into [`Case`]s. Everything that can be checked without running anything is
//! checked here, so that `everygraph-bench check` catches a broken spec before a
//! measurement window is spent on it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use super::template::Scope;
use super::units::{parse_duration, parse_size};

/// The spec format version this build reads.
pub const SPEC_SCHEMA: u32 = 1;

/// Placeholders the harness defines for every command.
pub const BUILTINS: &[&str] = &[
    "repo",
    "data",
    "work",
    "case_work",
    "dataset",
    "threads",
    "rep",
    "spec",
    "benchmark",
];

/// Metrics the harness measures itself, for every run.
pub const BUILTIN_METRICS: &[(&str, &str)] = &[
    ("wall_seconds", "s"),
    ("user_seconds", "s"),
    ("system_seconds", "s"),
    ("memory_peak_bytes", "B"),
    ("anon_peak_bytes", "B"),
    ("file_peak_bytes", "B"),
    ("pgmajfault", "faults"),
    ("refaults", "pages"),
    ("read_bytes", "B"),
    ("write_bytes", "B"),
    ("stop_seconds", "s"),
];

const DEFAULT_REPETITIONS: u32 = 3;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DEFAULT_SETTLE: Duration = Duration::from_secs(1);
const DEFAULT_SAMPLE: Duration = Duration::from_millis(20);
const DEFAULT_COLD_COPIES: u32 = 2;
const DEFAULT_TOLERANCE: f64 = 0.10;

/// A string, number or boolean in the TOML, kept as text (matrix values, conditions).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "ScalarRepr")]
pub struct Scalar(pub String);

#[derive(Deserialize)]
#[serde(untagged)]
enum ScalarRepr {
    S(String),
    I(i64),
    F(f64),
    B(bool),
}

impl From<ScalarRepr> for Scalar {
    fn from(r: ScalarRepr) -> Self {
        Scalar(match r {
            ScalarRepr::S(s) => s,
            ScalarRepr::I(i) => i.to_string(),
            ScalarRepr::F(f) => f.to_string(),
            ScalarRepr::B(b) => b.to_string(),
        })
    }
}

/// Page-cache state the measured command starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheMode {
    /// After an untimed warm-up run (by default): the dataset is in the page cache.
    Warm,
    /// The dataset's private copy evicted and checked out of the page cache before each
    /// repetition, rotating over copies so that no two consecutive runs read the same
    /// blocks (see [`super::cache`]).
    Cold,
}

/// Which output stream a metric is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    /// Standard output.
    #[default]
    Stdout,
    /// Standard error.
    Stderr,
}

/// How an array value read through a JSON pointer becomes one number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reduce {
    /// The smallest element.
    Min,
    /// The largest element.
    Max,
    /// The arithmetic mean.
    Mean,
    /// The median (mean of the two middle elements for an even count).
    Median,
    /// The sum.
    Sum,
}

/// The whole spec file as written.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecFile {
    /// Must be [`SPEC_SCHEMA`].
    pub schema: u32,
    /// Equal to the file name without `.toml`; results go to `bench/results/<name>/`.
    pub name: String,
    /// What the spec measures, for the report.
    pub description: String,
    /// Where the numbers it reproduces come from, if any.
    #[serde(default)]
    pub reference: Option<String>,
    /// Values every benchmark inherits.
    #[serde(default)]
    pub defaults: Defaults,
    /// Template variables usable in every command (`{glab}`).
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// Thresholds of the idle gate and of the disturbance check.
    #[serde(default)]
    pub gate: GateDef,
    /// Untimed steps run once before the benchmarks (builds, decompression).
    #[serde(default)]
    pub prepare: Vec<PrepareDef>,
    /// Files the commands run (templates): their SHA-256 goes into every row, so that a
    /// row names the binary that produced it, not only the revision.
    #[serde(default)]
    pub binaries: Vec<String>,
    /// The benchmarks.
    #[serde(default, rename = "benchmark")]
    pub benchmarks: Vec<BenchmarkDef>,
}

/// Defaults for every benchmark of the spec.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    /// Hard memory limit (`MemoryMax=`), like `8G`.
    pub memory_limit: Option<String>,
    /// Measured repetitions per case.
    pub repetitions: Option<u32>,
    /// Untimed runs before the measured ones (default 1 for warm cache, 0 for cold).
    pub warmup: Option<u32>,
    /// Page-cache state.
    pub cache: Option<CacheMode>,
    /// Private copies a cold-cache case rotates over (at least 2).
    pub cold_copies: Option<u32>,
    /// Thread counts; each makes a case.
    pub threads: Option<Vec<u32>>,
    /// A run longer than this is killed and recorded as timed out.
    pub timeout: Option<String>,
    /// Quiet window before each repetition, in which other activity is measured.
    pub settle: Option<String>,
    /// How often the memory of the run's cgroup is sampled for the anonymous and
    /// file-backed peaks.
    pub sample_interval: Option<String>,
    /// Empty the case's work directory before each repetition (default true).
    pub clean_work: Option<bool>,
}

/// Idle-gate thresholds.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateDef {
    /// An official run refuses to start above this 1-minute load average (default 1.0).
    pub max_load: Option<f64>,
    /// A repetition is disturbed when other processes used more than this many cores
    /// on average, in a settle window around it or while it ran (default 0.5).
    pub disturbed_above_cores: Option<f64>,
    /// … or moved more than this many MB/s to or from storage (default 50: background
    /// services stay far below it, a copy or another ingest far above).
    pub disturbed_above_io_mb_s: Option<f64>,
    /// An official run refuses to start with less `MemAvailable` than this (default:
    /// the largest memory limit of the cases selected).
    pub min_mem_available: Option<String>,
}

/// One untimed preparation step.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareDef {
    /// For messages.
    pub name: String,
    /// Program and arguments (templates).
    pub command: Option<Vec<String>>,
    /// Or a `sh -c` script (template).
    pub shell: Option<String>,
    /// Working directory (template).
    pub cwd: Option<String>,
    /// Skip the step when this path exists (template).
    pub creates: Option<String>,
    /// Hard memory limit; defaults to the spec's.
    pub memory_limit: Option<String>,
    /// Run the step only when a selected case belongs to one of these benchmarks
    /// (default: always).
    #[serde(default)]
    pub needed_by: Vec<String>,
}

/// One benchmark as written.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkDef {
    /// Unique within the spec.
    pub name: String,
    /// The report puts benchmarks of one group in one table.
    pub group: Option<String>,
    /// What it measures.
    pub description: Option<String>,
    /// A dataset of the registry, by name.
    pub dataset: Option<String>,
    /// Or a path under the data root (or absolute), like `csr/orkut`.
    pub dataset_path: Option<String>,
    /// Program and arguments (templates).
    pub command: Option<Vec<String>>,
    /// Or a `sh -c` script (template).
    pub shell: Option<String>,
    /// Working directory (template).
    pub cwd: Option<String>,
    /// Extra environment (values are templates).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Overrides of [`Defaults`].
    pub threads: Option<Vec<u32>>,
    /// Hard memory limit.
    pub memory_limit: Option<String>,
    /// Measured repetitions.
    pub repetitions: Option<u32>,
    /// Untimed runs first.
    pub warmup: Option<u32>,
    /// Page-cache state.
    pub cache: Option<CacheMode>,
    /// Private copies for cold cache.
    pub cold_copies: Option<u32>,
    /// Kill after this long.
    pub timeout: Option<String>,
    /// Empty the work directory before each repetition.
    pub clean_work: Option<bool>,
    /// Variables to take the cartesian product over (`algo = ["pr32", "wcc"]`).
    #[serde(default)]
    pub matrix: BTreeMap<String, Vec<Scalar>>,
    /// Combinations to leave out (`{ algo = "tc", threads = 1 }`).
    #[serde(default)]
    pub exclude: Vec<BTreeMap<String, Scalar>>,
    /// What the report prints next to every number: graph, precision, storage… (values
    /// are templates). Threads, memory limit and idleness are added by the harness.
    #[serde(default)]
    pub conditions: BTreeMap<String, Scalar>,
    /// Numbers read from the command's output.
    #[serde(default)]
    pub metric: Vec<MetricDef>,
    /// The metric the report and the run-to-run comparison lead with (default: the
    /// first metric, or `wall_seconds` without one).
    pub primary: Option<String>,
    /// Published values to compare with.
    #[serde(default)]
    pub reference: Vec<ReferenceDef>,
    /// Where the command's plan estimates and the actuals are (ux-contract P-7, P-9).
    pub plan: Option<PlanDef>,
    /// Send a signal after a delay and measure how long the command takes to stop
    /// (ux-contract C-4).
    pub interrupt: Option<InterruptDef>,
    /// The comparison runner this benchmark uses (`networkit`). A runner benchmark must
    /// record what ran: `tool`, `version`, `algorithm` and `options`, through `record`
    /// or `record_json`.
    pub runner: Option<String>,
    /// Text fields stored in every row (templates): what ran, for tools that do not
    /// print it themselves.
    #[serde(default)]
    pub record: BTreeMap<String, String>,
    /// Text fields stored in every row, read through JSON pointers from the last JSON
    /// object on standard output (an object or array is stored as its JSON text).
    #[serde(default)]
    pub record_json: BTreeMap<String, String>,
    /// Further files the command runs (templates), hashed into every row.
    #[serde(default)]
    pub binaries: Vec<String>,
    /// Conditions for some cases only: `{ match = { algo = "pr32" }, conditions = { precision = "f32" } }`.
    #[serde(default)]
    pub when: Vec<WhenDef>,
    /// Set when the benchmark cannot run on this machine: the reason. It is recorded
    /// as a skipped row, never silently dropped.
    pub unavailable: Option<String>,
    /// Why a warm case whose memory limit is below its dataset's size is right (it
    /// reads with `O_DIRECT`, or reads other files than the dataset): it silences the
    /// out-of-core lint of `check`.
    pub warm_ok: Option<String>,
}

/// Fields that the runner of a comparison benchmark must record (CLAUDE.md § 3: "check
/// what a reference tool actually ran").
pub const RUNNER_FIELDS: [&str; 4] = ["tool", "version", "algorithm", "options"];

/// Conditions that apply to the cases matching some matrix values (and `threads`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhenDef {
    /// Matrix values and `threads` to match.
    #[serde(rename = "match")]
    pub matches: BTreeMap<String, Scalar>,
    /// Conditions set (or replaced) for those cases.
    pub conditions: BTreeMap<String, Scalar>,
}

/// A number read from the output.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricDef {
    /// Its name in the rows and the report.
    pub name: String,
    /// A JSON pointer (RFC 6901) into the last line of the stream that is a JSON object.
    pub json: Option<String>,
    /// Or a regular expression whose first capture group is the number (first match).
    pub regex: Option<String>,
    /// The stream.
    #[serde(default)]
    pub from: Stream,
    /// For an array value: how to make one number of it.
    pub reduce: Option<Reduce>,
    /// Multiply by this (`1073741824` for GiB to bytes, `0.05` for 20 iterations to one).
    pub scale: Option<f64>,
    /// Unit after scaling (`s`, `B`, `edges/s`), for the report.
    pub unit: Option<String>,
    /// A missing value is recorded as absent instead of failing the row.
    #[serde(default)]
    pub optional: bool,
}

/// A published value to compare a case's median with.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceDef {
    /// Which cases it applies to: matrix values and `threads`. Empty: all of them.
    #[serde(default, rename = "match")]
    pub matches: BTreeMap<String, Scalar>,
    /// The metric compared.
    pub metric: String,
    /// The published value; the median must be within `tolerance` of it.
    pub value: Option<f64>,
    /// Or an upper bound the largest value must not exceed (the 2 s of C-4).
    pub max: Option<f64>,
    /// Relative tolerance for `value` (default 0.10).
    pub tolerance: Option<f64>,
    /// Where the value is published.
    pub source: String,
}

/// Where plan estimates and actuals come from.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDef {
    /// JSON pointer to the `estimate` object of the command's JSON document (default
    /// `/estimate`), in the last JSON line of standard output.
    pub estimate: Option<String>,
    /// For each of the four quantities, the metric (spec or built-in) that holds the
    /// actual. Defaults: `seconds` = `wall_seconds`, `peak_memory_bytes` =
    /// `memory_peak_bytes`, `scratch_bytes` and `output_bytes` = metrics of those names.
    #[serde(default)]
    pub actual: BTreeMap<String, String>,
}

/// A signal sent during the run.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterruptDef {
    /// Delay after the start.
    pub after: String,
    /// `INT` (default) or `TERM`.
    pub signal: Option<String>,
    /// The exit status a clean stop gives (default 128 + the signal number: 130 for
    /// INT, 143 for TERM, ux-contract C-7). Any other ending is not ok.
    pub expect_exit: Option<i32>,
}

/// The four quantities of an estimate (ux-contract § 3.3).
pub const PLAN_QUANTITIES: [&str; 4] = [
    "seconds",
    "peak_memory_bytes",
    "scratch_bytes",
    "output_bytes",
];

/// What a case runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Program {
    /// Program and arguments, each a template.
    Argv(Vec<String>),
    /// A `sh -c` script template.
    Shell(String),
}

/// Where a case's data is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatasetRef {
    /// A registry name.
    Registry(String),
    /// A path under the data root, or absolute.
    Path(String),
}

impl DatasetRef {
    /// How rows name it.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            DatasetRef::Registry(s) | DatasetRef::Path(s) => s,
        }
    }
}

/// A signal for [`Interrupt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopSignal {
    /// SIGINT, what Ctrl-C sends.
    Int,
    /// SIGTERM.
    Term,
}

impl StopSignal {
    /// `INT` or `TERM`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            StopSignal::Int => "INT",
            StopSignal::Term => "TERM",
        }
    }
}

/// A resolved [`InterruptDef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interrupt {
    /// Delay after the start.
    pub after: Duration,
    /// The signal.
    pub signal: StopSignal,
    /// The exit status of a clean stop.
    pub expect_exit: i32,
}

/// A resolved [`ReferenceDef`] that applies to one case.
#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    /// The metric.
    pub metric: String,
    /// Published value, or
    pub value: Option<f64>,
    /// upper bound.
    pub max: Option<f64>,
    /// Relative tolerance for `value`.
    pub tolerance: f64,
    /// Where it is published.
    pub source: String,
}

/// One benchmark at one thread count and one combination of matrix values.
#[derive(Debug, Clone)]
pub struct Case {
    /// `benchmark` alone, or `benchmark/key=value/…/t32`.
    pub id: String,
    /// The benchmark's name.
    pub benchmark: String,
    /// The report table it goes in.
    pub group: String,
    /// Matrix values (not `threads`).
    pub vars: BTreeMap<String, String>,
    /// Thread count.
    pub threads: u32,
    /// Hard memory limit in bytes.
    pub memory_limit: u64,
    /// Measured repetitions.
    pub repetitions: u32,
    /// Untimed runs first.
    pub warmup: u32,
    /// Page-cache state.
    pub cache: CacheMode,
    /// Private copies (cold cache).
    pub cold_copies: u32,
    /// Kill after this long.
    pub timeout: Duration,
    /// Quiet window before each repetition.
    pub settle: Duration,
    /// Memory sampling interval.
    pub sample_interval: Duration,
    /// Empty the work directory before each repetition.
    pub clean_work: bool,
    /// The data, if any.
    pub dataset: Option<DatasetRef>,
    /// What runs.
    pub program: Program,
    /// Working directory template.
    pub cwd: Option<String>,
    /// Extra environment templates.
    pub env: BTreeMap<String, String>,
    /// Condition templates.
    pub conditions: BTreeMap<String, String>,
    /// Metrics to extract.
    pub metrics: Vec<MetricDef>,
    /// Lead metric.
    pub primary: String,
    /// References that apply to this case.
    pub references: Vec<Reference>,
    /// Plan fields.
    pub plan: Option<PlanDef>,
    /// Signal to send.
    pub interrupt: Option<Interrupt>,
    /// Comparison runner.
    pub runner: Option<String>,
    /// Text fields recorded literally (templates).
    pub record: BTreeMap<String, String>,
    /// Text fields read through JSON pointers from the output.
    pub record_json: BTreeMap<String, String>,
    /// Files the command runs, the spec's and the benchmark's (templates).
    pub binaries: Vec<String>,
    /// Why it cannot run, if so.
    pub unavailable: Option<String>,
    /// Why warm is right below the dataset's size ([`BenchmarkDef::warm_ok`]).
    pub warm_ok: Option<String>,
}

/// A resolved preparation step.
#[derive(Debug, Clone)]
pub struct Prepare {
    /// For messages.
    pub name: String,
    /// What runs.
    pub program: Program,
    /// Working directory template.
    pub cwd: Option<String>,
    /// Skip when this exists (template).
    pub creates: Option<String>,
    /// Hard memory limit in bytes.
    pub memory_limit: u64,
    /// Benchmarks that need it; empty: all.
    pub needed_by: Vec<String>,
}

impl Prepare {
    /// Whether a run of `cases` needs this step.
    #[must_use]
    pub fn needed(&self, cases: &[&Case]) -> bool {
        self.needed_by.is_empty() || cases.iter().any(|c| self.needed_by.contains(&c.benchmark))
    }
}

/// Idle-gate thresholds, resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gate {
    /// Refuse an official run above this 1-minute load average.
    pub max_load: f64,
    /// Disturbed above this many busy cores of others.
    pub disturbed_above_cores: f64,
    /// Disturbed above this many MB/s of others' storage traffic.
    pub disturbed_above_io_mb_s: f64,
    /// Refuse an official run below this `MemAvailable`; `None`: the largest limit.
    pub min_mem_available: Option<u64>,
}

/// A checked spec.
#[derive(Debug, Clone)]
pub struct Spec {
    /// Name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Source of the reproduced numbers.
    pub reference: Option<String>,
    /// Variables.
    pub vars: BTreeMap<String, String>,
    /// Gate thresholds.
    pub gate: Gate,
    /// Preparation steps.
    pub prepare: Vec<Prepare>,
    /// All cases, in spec order.
    pub cases: Vec<Case>,
    /// SHA-256 of the file's bytes, recorded in every row.
    pub sha256: String,
    /// The file it was read from (`None` when parsed from text).
    pub path: Option<std::path::PathBuf>,
}

impl Spec {
    /// Reads and checks a spec file. Its name must equal the file name.
    ///
    /// # Errors
    ///
    /// The file cannot be read, is not valid TOML for this format, or fails a check.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        let mut spec = Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        spec.path = Some(path.to_path_buf());
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if spec.name != stem {
            return Err(format!(
                "{}: the spec is named {:?} but the file is {stem:?}.toml; make them equal",
                path.display(),
                spec.name
            ));
        }
        Ok(spec)
    }

    /// Parses and checks spec text.
    ///
    /// # Errors
    ///
    /// Invalid TOML for this format, or a failed check; the message names the place.
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: SpecFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let sha256 = crate::datasets::hex(&<sha2::Sha256 as sha2::Digest>::digest(text.as_bytes()));
        resolve(file, sha256)
    }

    /// The cases whose id contains one of `filters` (all cases when `filters` is empty).
    #[must_use]
    pub fn select(&self, filters: &[String]) -> Vec<&Case> {
        self.cases
            .iter()
            .filter(|c| filters.is_empty() || filters.iter().any(|f| c.id.contains(f.as_str())))
            .collect()
    }
}

fn check_name(what: &str, name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{what} {name:?} must be non-empty and use only a-z, 0-9, '-' and '_'"
        ))
    }
}

fn program(
    what: &str,
    command: Option<Vec<String>>,
    shell: Option<String>,
) -> Result<Program, String> {
    match (command, shell) {
        (Some(c), None) if !c.is_empty() => Ok(Program::Argv(c)),
        (None, Some(s)) if !s.trim().is_empty() => Ok(Program::Shell(s)),
        (Some(_), Some(_)) => Err(format!("{what}: give either command or shell, not both")),
        _ => Err(format!("{what}: give a non-empty command (or shell)")),
    }
}

fn limit_text<'a>(
    what: &str,
    own: Option<&'a String>,
    default: Option<&'a String>,
) -> Result<&'a String, String> {
    own.or(default).ok_or_else(|| {
        format!(
            "{what} has no memory_limit. Every run is under a hard memory limit \
             (CLAUDE.md § 7): add memory_limit = \"8G\" to it or to [defaults]"
        )
    })
}

fn limit(what: &str, own: Option<&String>, default: Option<&String>) -> Result<u64, String> {
    let text = limit_text(what, own, default)?;
    parse_size(text).map_err(|e| format!("{what}: memory_limit: {e}"))
}

fn duration(
    what: &str,
    field: &str,
    text: Option<&String>,
    default: Duration,
) -> Result<Duration, String> {
    text.map_or(Ok(default), |t| {
        parse_duration(t).map_err(|e| format!("{what}: {field}: {e}"))
    })
}

fn resolve(file: SpecFile, sha256: String) -> Result<Spec, String> {
    if file.schema != SPEC_SCHEMA {
        return Err(format!(
            "schema = {} but this everygraph-bench reads schema {SPEC_SCHEMA}",
            file.schema
        ));
    }
    check_name("spec name", &file.name)?;
    for v in file.vars.keys() {
        if BUILTINS.contains(&v.as_str()) {
            return Err(format!("[vars] {v} would hide the built-in {{{v}}}"));
        }
    }
    let d = &file.defaults;
    let gate = Gate {
        max_load: file.gate.max_load.unwrap_or(1.0),
        disturbed_above_cores: file.gate.disturbed_above_cores.unwrap_or(0.5),
        disturbed_above_io_mb_s: file.gate.disturbed_above_io_mb_s.unwrap_or(50.0),
        min_mem_available: file
            .gate
            .min_mem_available
            .as_deref()
            .map(parse_size)
            .transpose()
            .map_err(|e| format!("[gate] min_mem_available: {e}"))?,
    };
    if !(gate.max_load >= 0.0
        && gate.disturbed_above_cores >= 0.0
        && gate.disturbed_above_io_mb_s >= 0.0)
    {
        return Err("[gate] thresholds must be non-negative numbers".into());
    }

    let mut prepare = Vec::new();
    let mut seen = BTreeSet::new();
    for p in file.prepare {
        let what = format!("prepare step {:?}", p.name);
        if !seen.insert(p.name.clone()) {
            return Err(format!("{what} appears twice"));
        }
        prepare.push(Prepare {
            memory_limit: limit(&what, p.memory_limit.as_ref(), d.memory_limit.as_ref())?,
            program: program(&what, p.command, p.shell)?,
            name: p.name,
            cwd: p.cwd,
            creates: p.creates,
            needed_by: p.needed_by,
        });
    }

    let mut cases = Vec::new();
    let mut names = BTreeSet::new();
    for b in &file.benchmarks {
        check_name("benchmark name", &b.name)?;
        if !names.insert(b.name.clone()) {
            return Err(format!("benchmark {:?} appears twice", b.name));
        }
        cases.extend(
            expand_benchmark(b, d, &file.binaries)
                .map_err(|e| format!("benchmark {:?}: {e}", b.name))?,
        );
    }
    for p in &prepare {
        if let Some(n) = p.needed_by.iter().find(|n| !names.contains(*n)) {
            return Err(format!(
                "prepare step {:?}: needed_by names {n:?}, which is not a benchmark",
                p.name
            ));
        }
    }
    if cases.is_empty() {
        return Err("the spec has no [[benchmark]] (or every combination is excluded)".into());
    }
    let spec = Spec {
        name: file.name,
        description: file.description,
        reference: file.reference,
        vars: file.vars,
        gate,
        prepare,
        cases,
        sha256,
        path: None,
    };
    check_templates(&spec)?;
    Ok(spec)
}

fn expand_benchmark(
    b: &BenchmarkDef,
    d: &Defaults,
    spec_binaries: &[String],
) -> Result<Vec<Case>, String> {
    // May name matrix variables (`memory_limit = "{limit}"`); parsed per case below.
    let limit_template = limit_text("it", b.memory_limit.as_ref(), d.memory_limit.as_ref())?;
    let program = program("it", b.command.clone(), b.shell.clone())?;
    let repetitions = b
        .repetitions
        .or(d.repetitions)
        .unwrap_or(DEFAULT_REPETITIONS);
    if repetitions == 0 {
        return Err("repetitions must be at least 1".into());
    }
    let cache = b.cache.or(d.cache).unwrap_or(CacheMode::Warm);
    let warmup = b
        .warmup
        .or(d.warmup)
        .unwrap_or(u32::from(cache == CacheMode::Warm));
    let cold_copies = b
        .cold_copies
        .or(d.cold_copies)
        .unwrap_or(DEFAULT_COLD_COPIES);
    let dataset = match (&b.dataset, &b.dataset_path) {
        (Some(n), None) => Some(DatasetRef::Registry(n.clone())),
        (None, Some(p)) => Some(DatasetRef::Path(p.clone())),
        (None, None) => None,
        (Some(_), Some(_)) => return Err("give either dataset or dataset_path, not both".into()),
    };
    if cache == CacheMode::Cold {
        if dataset.is_none() {
            return Err("cache = \"cold\" needs a dataset (or dataset_path) to evict".into());
        }
        if cold_copies < 2 {
            return Err(
                "cold_copies must be at least 2: consecutive cold runs must not read the \
                 same blocks (the SSD keeps a cache of its own)"
                    .into(),
            );
        }
    }
    let threads = b
        .threads
        .clone()
        .or_else(|| d.threads.clone())
        .unwrap_or_else(|| vec![1]);
    if threads.is_empty() || threads.contains(&0) {
        return Err("threads must be a non-empty list of positive counts".into());
    }
    let timeout = duration(
        "it",
        "timeout",
        b.timeout.as_ref().or(d.timeout.as_ref()),
        DEFAULT_TIMEOUT,
    )?;
    let settle = duration("it", "settle", d.settle.as_ref(), DEFAULT_SETTLE)?;
    let sample_interval = duration(
        "it",
        "sample_interval",
        d.sample_interval.as_ref(),
        DEFAULT_SAMPLE,
    )?;
    if sample_interval < Duration::from_millis(1) {
        return Err("sample_interval must be at least 1ms".into());
    }

    for key in b.matrix.keys() {
        if BUILTINS.contains(&key.as_str()) {
            return Err(format!(
                "matrix key {key} would hide the built-in {{{key}}}"
            ));
        }
        if b.matrix[key].is_empty() {
            return Err(format!("matrix key {key} has no values"));
        }
    }
    let known_key = |k: &str| k == "threads" || b.matrix.contains_key(k);
    for ex in &b.exclude {
        if let Some(k) = ex.keys().find(|k| !known_key(k)) {
            return Err(format!(
                "exclude names {k}, which is neither threads nor a matrix key"
            ));
        }
    }

    // Metrics.
    let mut metric_names = BTreeSet::new();
    for m in &b.metric {
        check_name("metric name", &m.name)?;
        if BUILTIN_METRICS.iter().any(|(n, _)| *n == m.name) {
            return Err(format!(
                "metric {} is measured by the harness already",
                m.name
            ));
        }
        if !metric_names.insert(m.name.as_str()) {
            return Err(format!("metric {} appears twice", m.name));
        }
        match (&m.json, &m.regex) {
            (Some(p), None) => {
                if !(p.is_empty() || p.starts_with('/')) {
                    return Err(format!(
                        "metric {}: json = {p:?} is not a JSON pointer (it starts with '/')",
                        m.name
                    ));
                }
            }
            (None, Some(r)) => {
                let re = regex_lite::Regex::new(r)
                    .map_err(|e| format!("metric {}: regex: {e}", m.name))?;
                if re.captures_len() < 2 {
                    return Err(format!(
                        "metric {}: the regex needs a capture group around the number",
                        m.name
                    ));
                }
            }
            _ => return Err(format!("metric {}: give either json or regex", m.name)),
        }
    }
    let is_metric =
        |n: &str| metric_names.contains(n) || BUILTIN_METRICS.iter().any(|(b, _)| *b == n);
    let primary = b
        .primary
        .clone()
        .or_else(|| b.metric.first().map(|m| m.name.clone()))
        .unwrap_or_else(|| "wall_seconds".into());
    if !is_metric(&primary) {
        return Err(format!("primary = {primary:?} is not one of its metrics"));
    }
    for r in &b.reference {
        if !is_metric(&r.metric) {
            return Err(format!(
                "a reference names metric {:?}, which it does not have",
                r.metric
            ));
        }
        if let Some(k) = r.matches.keys().find(|k| !known_key(k)) {
            return Err(format!(
                "a reference matches on {k}, which is neither threads nor a matrix key"
            ));
        }
        if r.value.is_some() == r.max.is_some() {
            return Err(format!(
                "the reference for {} needs exactly one of value or max",
                r.metric
            ));
        }
    }
    if let Some(plan) = &b.plan {
        for (q, source) in &plan.actual {
            if !PLAN_QUANTITIES.contains(&q.as_str()) {
                return Err(format!("plan.actual.{q} is not one of {PLAN_QUANTITIES:?}"));
            }
            if !is_metric(source) {
                return Err(format!(
                    "plan.actual.{q} = {source:?} is not one of its metrics"
                ));
            }
        }
    }
    let interrupt = b
        .interrupt
        .as_ref()
        .map(|i| -> Result<Interrupt, String> {
            let (signal, number) = match i.signal.as_deref().unwrap_or("INT") {
                "INT" => (StopSignal::Int, 2),
                "TERM" => (StopSignal::Term, 15),
                other => return Err(format!("interrupt.signal = {other:?}: use INT or TERM")),
            };
            Ok(Interrupt {
                after: parse_duration(&i.after).map_err(|e| format!("interrupt.after: {e}"))?,
                signal,
                expect_exit: i.expect_exit.unwrap_or(128 + number),
            })
        })
        .transpose()?;
    if let Some(i) = interrupt {
        if i.after >= timeout {
            return Err("interrupt.after must be shorter than the timeout".into());
        }
    }

    if b.runner.is_some() {
        let missing: Vec<&str> = RUNNER_FIELDS
            .iter()
            .copied()
            .filter(|f| !b.record.contains_key(*f) && !b.record_json.contains_key(*f))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "a runner benchmark must record what ran: add {} to record_json (read from \
                 the runner's JSON line) or to record",
                missing.join(", ")
            ));
        }
    }
    if let Some(k) = b.record.keys().find(|k| b.record_json.contains_key(*k)) {
        return Err(format!("{k} is in both record and record_json"));
    }
    if let Some((k, p)) = b
        .record_json
        .iter()
        .find(|(_, p)| !(p.is_empty() || p.starts_with('/')))
    {
        return Err(format!(
            "record_json.{k} = {p:?} is not a JSON pointer (it starts with '/')"
        ));
    }
    for w in &b.when {
        if let Some(k) = w.matches.keys().find(|k| !known_key(k)) {
            return Err(format!(
                "a when entry matches on {k}, which is neither threads nor a matrix key"
            ));
        }
    }

    // The cartesian product of the matrix, then threads.
    let mut combos: Vec<BTreeMap<String, String>> = vec![BTreeMap::new()];
    for (key, values) in &b.matrix {
        combos = combos
            .into_iter()
            .flat_map(|c| {
                values.iter().map(move |v| {
                    let mut c = c.clone();
                    c.insert(key.clone(), v.0.clone());
                    c
                })
            })
            .collect();
    }
    let mut out = Vec::new();
    for vars in combos {
        for &t in &threads {
            let t_text = t.to_string();
            let value_of = |k: &str| {
                if k == "threads" {
                    Some(t_text.as_str())
                } else {
                    vars.get(k).map(String::as_str)
                }
            };
            let matches = |m: &BTreeMap<String, Scalar>| {
                m.iter().all(|(k, v)| value_of(k) == Some(v.0.as_str()))
            };
            if b.exclude.iter().any(matches) {
                continue;
            }
            let mut scope = Scope::default();
            // A dataset may live in the work directory, known only at run time:
            // `{work}` stays in the path for `run::dataset_path`.
            scope.set("work", "{work}");
            scope.set("threads", t_text.clone());
            for (k, v) in &vars {
                scope.set(k, v.clone());
            }
            let memory_limit = scope
                .expand(limit_template)
                .and_then(|text| parse_size(&text))
                .map_err(|e| format!("memory_limit: {e}"))?;
            let mut id = b.name.clone();
            for (k, v) in &vars {
                id.push_str(&format!("/{k}={v}"));
            }
            if threads.len() > 1 || !vars.is_empty() {
                id.push_str(&format!("/t{t}"));
            }
            out.push(Case {
                id,
                benchmark: b.name.clone(),
                group: b.group.clone().unwrap_or_else(|| b.name.clone()),
                vars: vars.clone(),
                threads: t,
                memory_limit,
                repetitions,
                warmup,
                cache,
                cold_copies,
                timeout,
                settle,
                sample_interval,
                clean_work: b.clean_work.or(d.clean_work).unwrap_or(true),
                // Like the limit, the name may use matrix values (`raw/com-{graph}…`).
                dataset: dataset
                    .as_ref()
                    .map(|d| -> Result<DatasetRef, String> {
                        Ok(match d {
                            DatasetRef::Registry(n) => DatasetRef::Registry(scope.expand(n)?),
                            DatasetRef::Path(p) => DatasetRef::Path(scope.expand(p)?),
                        })
                    })
                    .transpose()
                    .map_err(|e| format!("dataset: {e}"))?,
                program: program.clone(),
                cwd: b.cwd.clone(),
                env: b.env.clone(),
                conditions: b
                    .conditions
                    .iter()
                    .chain(
                        b.when
                            .iter()
                            .filter(|w| matches(&w.matches))
                            .flat_map(|w| w.conditions.iter()),
                    )
                    .map(|(k, v)| (k.clone(), v.0.clone()))
                    .collect(),
                record: b.record.clone(),
                record_json: b.record_json.clone(),
                binaries: spec_binaries.iter().chain(&b.binaries).cloned().collect(),
                metrics: b.metric.clone(),
                primary: primary.clone(),
                references: b
                    .reference
                    .iter()
                    .filter(|r| matches(&r.matches))
                    .map(|r| Reference {
                        metric: r.metric.clone(),
                        value: r.value,
                        max: r.max,
                        tolerance: r.tolerance.unwrap_or(DEFAULT_TOLERANCE),
                        source: r.source.clone(),
                    })
                    .collect(),
                plan: b.plan.clone(),
                interrupt,
                runner: b.runner.clone(),
                unavailable: b.unavailable.clone(),
                warm_ok: b.warm_ok.clone(),
            });
        }
    }
    Ok(out)
}

/// Built-ins defined for preparation steps, which belong to no case.
pub const PREPARE_BUILTINS: &[&str] = &["repo", "data", "work", "spec"];

/// A scope with the built-ins set to stand-ins, for checking templates.
fn check_scope(spec: &Spec, case: Option<&Case>) -> Scope {
    let mut scope = Scope::new(spec.vars.clone());
    let builtins = if case.is_some() {
        BUILTINS
    } else {
        PREPARE_BUILTINS
    };
    for b in builtins {
        scope.set(b, format!("<{b}>"));
    }
    if let Some(c) = case {
        for (k, v) in &c.vars {
            scope.set(k, v.clone());
        }
    }
    scope
}

fn check_program(scope: &Scope, p: &Program) -> Result<(), String> {
    match p {
        Program::Argv(args) => args.iter().try_for_each(|a| scope.expand(a).map(drop)),
        Program::Shell(s) => scope.expand(s).map(drop),
    }
}

fn check_templates(spec: &Spec) -> Result<(), String> {
    for p in &spec.prepare {
        let scope = check_scope(spec, None);
        let what = |e| format!("prepare step {:?}: {e}", p.name);
        check_program(&scope, &p.program).map_err(what)?;
        for t in p.cwd.iter().chain(&p.creates) {
            scope.expand(t).map_err(what)?;
        }
    }
    for c in &spec.cases {
        let scope = check_scope(spec, Some(c));
        let what = |e| format!("case {}: {e}", c.id);
        check_program(&scope, &c.program).map_err(what)?;
        for t in c
            .cwd
            .iter()
            .chain(c.env.values())
            .chain(c.conditions.values())
            .chain(c.record.values())
            .chain(&c.binaries)
        {
            scope.expand(t).map_err(what)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
schema = 1
name = "demo"
description = "a demo"
[defaults]
memory_limit = "1G"
[[benchmark]]
name = "sleep"
command = ["sleep", "0"]
"#;

    fn err(text: &str) -> String {
        Spec::parse(text).expect_err("must be rejected")
    }

    #[test]
    fn a_minimal_spec_has_one_case_with_the_defaults() {
        let spec = Spec::parse(MINIMAL).expect("valid");
        assert_eq!(spec.cases.len(), 1);
        let c = &spec.cases[0];
        assert_eq!(c.id, "sleep");
        assert_eq!(c.memory_limit, 1 << 30);
        assert_eq!(c.repetitions, 3);
        assert_eq!((c.cache, c.warmup), (CacheMode::Warm, 1));
        assert_eq!(c.primary, "wall_seconds");
        assert_eq!(spec.gate.max_load, 1.0);
        assert_eq!(spec.sha256.len(), 64);
    }

    #[test]
    fn a_spec_without_a_memory_limit_is_an_error() {
        let text = MINIMAL.replace("memory_limit = \"1G\"", "");
        assert!(err(&text).contains("has no memory_limit"));
        let text = MINIMAL.replace("\"1G\"", "\"lots\"");
        assert!(err(&text).contains("not a size"));
    }

    #[test]
    fn matrix_threads_and_exclusions_expand_in_order() {
        let text = format!(
            "{MINIMAL}\n{}",
            r#"
[[benchmark]]
name = "k"
group = "kernels"
threads = [1, 32]
matrix = { algo = ["pr32", "tc"], iters = [20] }
exclude = [{ algo = "tc", threads = 1 }]
command = ["glab", "--algo", "{algo}", "--iters", "{iters}", "--threads", "{threads}"]
conditions = { precision = "f32", graph = "orkut" }
metric = [{ name = "seconds", json = "/seconds_best", unit = "s" }]
reference = [
  { match = { algo = "pr32", threads = 32 }, metric = "seconds", value = 0.38, source = "FINDINGS" },
]
"#
        );
        let spec = Spec::parse(&text).expect("valid");
        let ids: Vec<&str> = spec.cases.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "sleep",
                "k/algo=pr32/iters=20/t1",
                "k/algo=pr32/iters=20/t32",
                "k/algo=tc/iters=20/t32"
            ]
        );
        let pr32 = &spec.cases[2];
        assert_eq!(pr32.primary, "seconds");
        assert_eq!(pr32.references.len(), 1);
        assert_eq!(pr32.references[0].tolerance, 0.10);
        assert!(spec.cases[1].references.is_empty());
        assert_eq!(spec.select(&["algo=tc".into()]).len(), 1);
    }

    #[test]
    fn a_memory_limit_may_come_from_the_matrix() {
        let text = format!(
            "{MINIMAL}\n{}",
            r#"
[[benchmark]]
name = "semiext"
matrix = { limit = ["16G", "2G"] }
memory_limit = "{limit}"
command = ["x"]
"#
        );
        let spec = Spec::parse(&text).expect("valid");
        let limits: Vec<u64> = spec.cases[1..].iter().map(|c| c.memory_limit).collect();
        assert_eq!(limits, [16 << 30, 2 << 30]);
        let bad = text.replace("\"{limit}\"", "\"{nope}\"");
        assert!(err(&bad).contains("memory_limit"), "{}", err(&bad));
    }

    #[test]
    fn mistakes_are_named() {
        let bench = |body: &str| format!("{MINIMAL}\n[[benchmark]]\nname = \"b\"\n{body}\n");
        let cases = [
            ("command = [\"x\"]\nshell = \"x\"", "not both"),
            ("command = []", "non-empty command"),
            ("command = [\"x\", \"{nope}\"]", "{nope}"),
            ("command = [\"x\"]\ncache = \"cold\"", "needs a dataset"),
            (
                "command = [\"x\"]\ncache = \"cold\"\ndataset = \"d\"\ncold_copies = 1",
                "at least 2",
            ),
            ("command = [\"x\"]\nthreads = []", "non-empty list"),
            ("command = [\"x\"]\nrepetitions = 0", "at least 1"),
            (
                "command = [\"x\"]\nmetric = [{ name = \"s\", json = \"seconds\" }]",
                "JSON pointer",
            ),
            (
                "command = [\"x\"]\nmetric = [{ name = \"s\", regex = \"[0-9]+\" }]",
                "capture group",
            ),
            (
                "command = [\"x\"]\nmetric = [{ name = \"wall_seconds\", json = \"/x\" }]",
                "measured by the harness",
            ),
            (
                "command = [\"x\"]\nprimary = \"speed\"",
                "not one of its metrics",
            ),
            (
                "command = [\"x\"]\nexclude = [{ algo = \"tc\" }]",
                "neither threads nor a matrix key",
            ),
            (
                "command = [\"x\"]\nmatrix = { threads = [1] }",
                "hide the built-in",
            ),
            ("command = [\"x\"]\nbogus = 1", "unknown field"),
            (
                "command = [\"x\"]\ninterrupt = { after = \"1s\", signal = \"HUP\" }",
                "INT or TERM",
            ),
            (
                "command = [\"x\"]\nplan = { actual = { seconds = \"nope\" } }",
                "not one of its metrics",
            ),
            (
                "command = [\"x\"]\nreference = [{ metric = \"wall_seconds\", source = \"s\" }]",
                "exactly one of value or max",
            ),
        ];
        for (body, expected) in cases {
            let e = err(&bench(body));
            assert!(e.contains(expected), "{body:?}: {e}");
        }
        assert!(
            err(&format!(
                "{MINIMAL}\n[[benchmark]]\nname = \"sleep\"\ncommand = [\"x\"]"
            ))
            .contains("twice")
        );
        assert!(err(&MINIMAL.replace("schema = 1", "schema = 2")).contains("schema"));
        assert!(err(&MINIMAL.replace("\"demo\"", "\"Demo\"")).contains("a-z"));
    }

    #[test]
    fn vars_may_use_builtins_but_not_hide_them() {
        let ok = MINIMAL
            .replace(
                "[defaults]",
                "[vars]\nglab = \"{repo}/lab/glab\"\n[defaults]",
            )
            .replace("[\"sleep\", \"0\"]", "[\"{glab}\"]");
        Spec::parse(&ok).expect("valid");
        let bad = MINIMAL.replace("[defaults]", "[vars]\nrepo = \"x\"\n[defaults]");
        assert!(err(&bad).contains("hide the built-in"));
    }
}
