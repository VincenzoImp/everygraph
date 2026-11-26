// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `everygraph-bench` command: datasets (issue #5) and the benchmark harness
//! (issue #4).

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line program talks to its user on the standard streams"
)]
#![allow(
    clippy::doc_markdown,
    reason = "doc comments on the clap types are the --help text, where backticks would print as such"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};

use everygraph_bench::datasets::{
    self, DataRoot, FetchOptions, FileStatus, Outcome, Registry, human_bytes, markdown,
};
use everygraph_bench::harness::compare::{self, CompareOptions};
use everygraph_bench::harness::measure::{MeasureOptions, measure};
use everygraph_bench::harness::procfs::Host;
use everygraph_bench::harness::run::{
    self, Context, RunError, RunOptions, dataset_path, path_safe,
};
use everygraph_bench::harness::spec::{Interrupt, Program, Spec, StopSignal};
use everygraph_bench::harness::template::Scope;
use everygraph_bench::harness::units::human_size;
use everygraph_bench::harness::{cache, lock, report, row, selftest, units};

/// `println!` that ignores a closed standard output (`everygraph-bench list | head`)
/// instead of panicking.
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ignored = writeln!(std::io::stdout(), $($arg)*);
    }};
}

/// Exit status of a run refused before it started (lock held, machine busy).
const REFUSED: u8 = 3;

#[derive(Parser, Debug)]
#[command(
    name = "everygraph-bench",
    about = "Datasets and the benchmark harness of Everygraph.",
    after_help = "\
Exit status: 0 success, 1 failure, 2 usage error, 3 an official run refused to start \
(another official run holds the lock, the machine is not idle, or the tree is dirty).

The data root is --data, else $EVERYGRAPH_DATA, else ./data. Specs live in \
bench/specs/; the format and the measurement protocol are in bench/README.md.",
    arg_required_else_help = true
)]
struct Cli {
    /// Dataset registry to use instead of the datasets/datasets.toml built in.
    #[arg(long, global = true, value_name = "FILE", value_parser = non_empty_path)]
    registry: Option<PathBuf>,
    /// Data root (default: $EVERYGRAPH_DATA, else ./data).
    #[arg(long, global = true, value_name = "DIR", value_parser = non_empty_path)]
    data: Option<PathBuf>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Download (or generate) and verify datasets.
    Fetch(FetchArgs),
    /// Re-read present files of datasets and check them.
    Verify {
        /// A dataset name, tier1, tier2, tier3 or all (tierN includes the lower tiers).
        #[arg(required = true, value_name = "NAME|TIER")]
        names: Vec<String>,
    },
    /// Show the datasets and what is on disk.
    List {
        /// Print the tables of datasets/README.md instead.
        #[arg(long)]
        markdown: bool,
        /// Replace the generated part of FILE with those tables.
        #[arg(long, value_name = "FILE", value_parser = non_empty_path)]
        write_readme: Option<PathBuf>,
    },
    /// Print the path of each file of a dataset.
    Path {
        /// The dataset.
        name: String,
    },
    /// Run a benchmark spec and record one JSON row per repetition.
    Run(RunArgs),
    /// Check specs and list their cases, without running anything.
    Check(CheckArgs),
    /// Markdown tables (and chart-ready JSON) from result files.
    Report(ReportArgs),
    /// Check, on this machine, that the harness does not see its own load as others':
    /// a busy loop on every CPU and a 1 GiB write, each 3 times. Run it first in an
    /// idle window.
    Selftest(SelftestArgs),
    /// Delete the work directory (builds, inputs, cold-cache copies, logs).
    Clean(CleanArgs),
    /// Runs inside the systemd-run scope of one repetition (used by `run`).
    #[command(hide = true)]
    Measure(MeasureArgs),
    /// The load of the self-test.
    #[command(hide = true)]
    Burn(BurnArgs),
}

#[derive(Args, Debug)]
struct SelftestArgs {
    /// Seconds of each busy-loop repetition.
    #[arg(long, value_name = "S", default_value_t = 10)]
    seconds: u32,
    /// Busy threads (default: every logical CPU).
    #[arg(long, value_name = "N")]
    threads: Option<u32>,
    #[command(flatten)]
    place: Where,
}

#[derive(Args, Debug)]
struct CleanArgs {
    /// Only this spec's part of the work directory (its cases, logs and prepared files
    /// under work/<spec>); shared inputs and cold-cache copies stay.
    #[arg(long, value_name = "SPEC")]
    spec: Option<String>,
    /// Say what would be deleted, and delete nothing.
    #[arg(long)]
    dry_run: bool,
    #[command(flatten)]
    place: Where,
}

#[derive(Args, Debug)]
struct BurnArgs {
    #[arg(long)]
    threads: u32,
    #[arg(long)]
    seconds: u64,
    #[arg(long, default_value_t = 0)]
    write_mib: u64,
    #[arg(long)]
    dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct FetchArgs {
    /// A dataset name, tier1, tier2, tier3 or all (tierN includes the lower tiers).
    #[arg(required = true, value_name = "NAME|TIER")]
    names: Vec<String>,
    /// Parallel connections per file.
    #[arg(long, value_name = "N", default_value_t = 8, value_parser = clap::value_parser!(u32).range(1..=64))]
    connections: u32,
    /// Copy files of the same name from DIR if they match.
    #[arg(long, value_name = "DIR", value_parser = non_empty_path)]
    from_dir: Option<PathBuf>,
    /// Also fetch files the registry pins no checksum for.
    #[arg(long)]
    allow_unpinned: bool,
    /// Delete placed files that no longer match, and fetch them again.
    #[arg(long)]
    replace: bool,
}

#[derive(Args, Debug)]
struct Where {
    /// The Everygraph repository (default: the git work tree around the current
    /// directory). Specs are read from, and results written to, its bench/ directory.
    #[arg(long, value_name = "DIR", value_parser = non_empty_path)]
    repo: Option<PathBuf>,
    /// Work directory for builds, inputs, cold-cache copies and logs (default:
    /// $EVERYGRAPH_BENCH_WORK, else bench/work in the repository). Not on tmpfs.
    #[arg(long, value_name = "DIR", value_parser = non_empty_path)]
    work: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct RunArgs {
    /// A spec name (bench/specs/NAME.toml) or the path of a spec file.
    spec: String,
    /// An official run: takes the lock (a second one fails at once), refuses a busy
    /// machine or a dirty tree, and writes to bench/results/ (committed). Without it the
    /// run is exploratory and writes to bench/exploratory/ (ignored by git).
    #[arg(long)]
    official: bool,
    /// Only cases whose id contains PATTERN (repeatable).
    #[arg(long = "only", value_name = "PATTERN")]
    only: Vec<String>,
    /// Repetitions of every case, instead of the spec's.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    repetitions: Option<u32>,
    /// Refuse an official run above this 1-minute load average (default: the spec's,
    /// else 1.0).
    #[arg(long, value_name = "LOAD")]
    max_load: Option<f64>,
    /// Let an official run start with uncommitted changes (every row says so).
    #[arg(long)]
    allow_dirty: bool,
    /// Run the preparation steps and make the cold-cache copies, then stop.
    #[arg(long, conflicts_with = "skip_prepare")]
    prepare_only: bool,
    /// Do not run the preparation steps.
    #[arg(long)]
    skip_prepare: bool,
    /// Write the rows to FILE instead of the default results file.
    #[arg(long, value_name = "FILE", value_parser = non_empty_path)]
    out: Option<PathBuf>,
    /// Run in rounds over the selected cases, in a seeded random order that the next
    /// round reverses (every row records its round and place).
    #[arg(long)]
    interleave: bool,
    /// The seed of --interleave (default: from the clock; the rows record it).
    #[arg(long, value_name = "S", requires = "interleave")]
    seed: Option<u64>,
    /// After the run, compare the cases with VAR=VALUE against those that differ from
    /// them only in VAR: ratios with 95% intervals (an A/B in one command).
    #[arg(long, value_name = "VAR=VALUE", value_parser = var_value)]
    compare_by: Option<(String, String)>,
    #[command(flatten)]
    place: Where,
}

fn var_value(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((k, v)) if !k.is_empty() && !v.is_empty() => Ok((k.into(), v.into())),
        _ => Err("expected VAR=VALUE, a matrix variable and its baseline value".into()),
    }
}

#[derive(Args, Debug)]
struct CheckArgs {
    /// Spec names or files (default: every spec under bench/specs/).
    specs: Vec<String>,
    /// Also print each case's command, expanded for this machine.
    #[arg(long)]
    commands: bool,
    #[command(flatten)]
    place: Where,
}

#[derive(Args, Debug)]
struct ReportArgs {
    /// Result files (JSON lines).
    #[arg(required = true, value_name = "RESULTS")]
    files: Vec<PathBuf>,
    /// Only cases whose id contains PATTERN (repeatable).
    #[arg(long = "only", value_name = "PATTERN")]
    only: Vec<String>,
    /// Judge the rows against the current references of this spec (a name under
    /// bench/specs/ or a file), not the ones they ran with; the report says so.
    #[arg(long, value_name = "SPEC")]
    spec: Option<String>,
    /// Also write chart-ready JSON to FILE.
    #[arg(long, value_name = "FILE", value_parser = non_empty_path)]
    json: Option<PathBuf>,
    /// The relative half-width that the "repetitions needed" column aims at.
    #[arg(long, value_name = "FRACTION", default_value_t = report::TARGET_HALF_WIDTH)]
    target: f64,
    /// Compare primary metrics instead, as ratios of medians with 95% intervals: each
    /// later run against the first, or within each run with --by. Exit status 1 when an
    /// interval leaves 1 ± δ.
    #[arg(long)]
    compare: bool,
    /// δ of every case without one in --deltas.
    #[arg(
        long,
        value_name = "FRACTION",
        default_value_t = 0.05,
        requires = "compare"
    )]
    max_diff: f64,
    /// Compare within each run: cases with VAR=VALUE against those that differ from
    /// them only in VAR.
    #[arg(long, value_name = "VAR=VALUE", value_parser = var_value, requires = "compare")]
    by: Option<(String, String)>,
    /// δ by case id: a JSON object written by --write-deltas from an A/A comparison.
    #[arg(long, value_name = "FILE", requires = "compare")]
    deltas: Option<PathBuf>,
    /// Write the smallest δ each case's interval meets, as JSON: run it on an A/A
    /// comparison (two runs of one revision) to calibrate δ.
    #[arg(long, value_name = "FILE", value_parser = non_empty_path, requires = "compare")]
    write_deltas: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct MeasureArgs {
    /// Where to write the measurement (JSON).
    #[arg(long, value_name = "FILE")]
    report: PathBuf,
    /// Memory sampling interval.
    #[arg(long, value_name = "MS", default_value_t = 20)]
    sample_interval_ms: u64,
    /// Kill the run after this long.
    #[arg(long, value_name = "MS")]
    timeout_ms: u64,
    /// Send --interrupt-signal after this long.
    #[arg(long, value_name = "MS")]
    interrupt_after_ms: Option<u64>,
    /// INT or TERM.
    #[arg(long, value_name = "SIGNAL", default_value = "INT")]
    interrupt_signal: String,
    /// The command.
    #[arg(last = true, required = true, value_name = "COMMAND")]
    command: Vec<String>,
}

fn non_empty_path(s: &str) -> Result<PathBuf, String> {
    if s.is_empty() {
        // An empty data root would silently mean the working directory.
        Err("needs a non-empty value".into())
    } else {
        Ok(PathBuf::from(s))
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("everygraph-bench: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cli: Cli) -> Result<ExitCode, String> {
    let registry = || -> Result<Registry, String> {
        match &cli.registry {
            Some(path) => Registry::load(path),
            None => Registry::embedded(),
        }
        .map_err(|e| e.to_string())
    };
    let root = cli
        .data
        .clone()
        .map_or_else(DataRoot::from_env, DataRoot::new);
    match cli.command {
        Cmd::Fetch(args) => fetch(&registry()?, &root, &args),
        Cmd::Verify { names } => verify(&registry()?, &root, &names),
        Cmd::List {
            markdown,
            write_readme,
        } => list(&registry()?, &root, markdown, write_readme.as_deref()),
        Cmd::Path { name } => {
            let registry = registry()?;
            let ds = registry
                .get(&name)
                .ok_or_else(|| format!("no dataset named {name:?}"))?;
            for f in &ds.files {
                out!("{}", root.file_path(ds, f).display());
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Run(args) => run_spec(registry()?, root, &args),
        Cmd::Check(args) => check(registry()?, root, &args),
        Cmd::Report(args) => report_files(&args),
        Cmd::Measure(args) => measure_command(&args),
        Cmd::Selftest(args) => selftest(registry()?, root, &args),
        Cmd::Clean(args) => clean(registry()?, root, &args),
        Cmd::Burn(args) => {
            selftest::burn(
                args.threads,
                args.seconds,
                args.write_mib,
                args.dir.as_deref(),
            )?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

// ---------------------------------------------------------------- datasets

fn selection<'r>(
    registry: &'r Registry,
    operands: &[String],
) -> Result<Vec<&'r datasets::Dataset>, String> {
    let mut out: Vec<&datasets::Dataset> = Vec::new();
    for op in operands {
        for ds in registry.select(op).map_err(|e| e.to_string())? {
            if !out.iter().any(|d| d.name == ds.name) {
                out.push(ds);
            }
        }
    }
    Ok(out)
}

fn fetch(registry: &Registry, root: &DataRoot, args: &FetchArgs) -> Result<ExitCode, String> {
    let selected = selection(registry, &args.names)?;
    let mut options = FetchOptions {
        allow_unpinned: args.allow_unpinned,
        replace: args.replace,
        from_dir: args.from_dir.clone(),
        ..FetchOptions::default()
    };
    options.download.connections = usize::try_from(args.connections).map_err(|e| e.to_string())?;
    let agent = datasets::agent(&options.download).map_err(|e| e.to_string())?;
    let mut stderr = std::io::stderr();
    let mut failed = 0;
    eprintln!("data root: {}", root.path().display());
    for ds in &selected {
        let size = ds
            .total_size()
            .map_or_else(|| "size unknown".to_string(), human_bytes);
        eprintln!("{} (tier {}, {size})", ds.name, ds.tier);
        match datasets::fetch(root, &agent, ds, &options, &mut stderr) {
            Ok(outcomes) => {
                for (f, o) in ds.files.iter().zip(&outcomes) {
                    let what = match o {
                        Outcome::AlreadyVerified => "already verified".to_string(),
                        Outcome::VerifiedInPlace => "verified in place".to_string(),
                        Outcome::Placed { from } => format!("verified, from {from}"),
                        Outcome::PlacedUnpinned { from, sha256 } => format!(
                            "placed UNPINNED from {from}; sha256 = \"{sha256}\" (add it to the registry)"
                        ),
                    };
                    eprintln!("  {}: {what}", f.name);
                }
            }
            Err(e) => {
                eprintln!("  FAILED: {e}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        return Err(format!("{failed} of {} datasets failed", selected.len()));
    }
    Ok(ExitCode::SUCCESS)
}

fn verify(registry: &Registry, root: &DataRoot, names: &[String]) -> Result<ExitCode, String> {
    let selected = selection(registry, names)?;
    let mut failed = 0;
    for ds in &selected {
        for f in &ds.files {
            match datasets::verify_file(root, ds, f) {
                Ok(status) => out!("{}/{}: {}", ds.name, f.name, status.label()),
                Err(e) => {
                    out!("{}/{}: FAILED: {e}", ds.name, f.name);
                    failed += 1;
                }
            }
        }
    }
    if failed > 0 {
        return Err(format!("{failed} files failed verification"));
    }
    Ok(ExitCode::SUCCESS)
}

fn list(
    registry: &Registry,
    root: &DataRoot,
    as_markdown: bool,
    write_readme: Option<&Path>,
) -> Result<ExitCode, String> {
    if let Some(path) = write_readme {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        let updated = markdown::splice(&text, &markdown::render(registry))
            .ok_or_else(|| format!("{} has no generated-section markers", path.display()))?;
        std::fs::write(path, updated).map_err(|e| format!("writing {}: {e}", path.display()))?;
        return Ok(ExitCode::SUCCESS);
    }
    if as_markdown {
        out!("{}", markdown::render(registry).trim_end());
        return Ok(ExitCode::SUCCESS);
    }
    out!("data root: {}", root.path().display());
    out!(
        "{:<32} {:>4}  {:<16} {:>10}  status",
        "name",
        "tier",
        "family",
        "download"
    );
    for ds in &registry.datasets {
        let statuses: Vec<FileStatus> = ds.files.iter().map(|f| root.status(ds, f)).collect();
        let size = if ds.is_generated() {
            "generated".to_string()
        } else {
            ds.total_size().map_or_else(|| "?".to_string(), human_bytes)
        };
        out!(
            "{:<32} {:>4}  {:<16} {:>10}  {}",
            ds.name,
            ds.tier,
            ds.family.as_str(),
            size,
            summarize(&statuses)
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn summarize(statuses: &[FileStatus]) -> String {
    if statuses.iter().all(|s| *s == FileStatus::Verified) {
        return "verified".into();
    }
    if statuses.iter().all(|s| *s == FileStatus::Missing) {
        return "missing".into();
    }
    if let [one] = statuses {
        return one.label();
    }
    statuses
        .iter()
        .map(FileStatus::label)
        .collect::<Vec<_>>()
        .join(" / ")
}

// ---------------------------------------------------------------- harness

fn find_repo(given: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = given {
        return Ok(p.to_path_buf());
    }
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("running git to find the repository: {e}; pass --repo"))?;
    if !out.status.success() {
        return Err(
            "not inside a git work tree; run from the Everygraph repository or pass --repo".into(),
        );
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

fn spec_path(repo: &Path, name: &str) -> PathBuf {
    if name.ends_with(".toml") || name.contains('/') {
        PathBuf::from(name)
    } else {
        repo.join("bench")
            .join("specs")
            .join(format!("{name}.toml"))
    }
}

fn context(registry: Registry, data: DataRoot, place: &Where) -> Result<Context, String> {
    let repo = find_repo(place.repo.as_deref())?;
    let work = place
        .work
        .clone()
        .or_else(|| {
            std::env::var_os("EVERYGRAPH_BENCH_WORK")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            std::env::var_os("EVERYGRAPH_SCRATCH")
                .filter(|v| !v.is_empty())
                .map(|s| PathBuf::from(s).join("bench-work"))
        })
        .unwrap_or_else(|| repo.join("bench").join("work"));
    let work = std::path::absolute(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let data = DataRoot::new(std::path::absolute(data.path()).map_err(|e| e.to_string())?);
    Ok(Context {
        repo,
        data,
        registry,
        work,
        host: Host::default(),
        exe: std::env::current_exe().map_err(|e| format!("finding this executable: {e}"))?,
    })
}

fn run_spec(registry: Registry, data: DataRoot, args: &RunArgs) -> Result<ExitCode, String> {
    let ctx = context(registry, data, &args.place)?;
    let spec = Spec::load(&spec_path(&ctx.repo, &args.spec))?;
    let opts = RunOptions {
        official: args.official,
        filters: args.only.clone(),
        repetitions: args.repetitions,
        max_load: args.max_load,
        allow_dirty: args.allow_dirty,
        prepare_only: args.prepare_only,
        skip_prepare: args.skip_prepare,
        out: args.out.clone(),
        lock_path: lock::default_path(),
        interleave: args.interleave.then(|| {
            args.seed.unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() ^ u64::from(d.subsec_nanos()) << 32)
            })
        }),
    };
    let mut stderr = std::io::stderr();
    match run::run(&spec, &ctx, &opts, &mut stderr) {
        Err(RunError::Refused(msg)) => {
            eprintln!("everygraph-bench: refused: {msg}");
            Ok(ExitCode::from(REFUSED))
        }
        Err(RunError::Failed(msg)) => Err(msg),
        Ok(outcome) => {
            if let Some(path) = &outcome.results {
                let rows = row::read_rows(path)?;
                out!(
                    "{}",
                    report::markdown(&rows, report::TARGET_HALF_WIDTH).trim_end()
                );
                if let Some(by) = &args.compare_by {
                    let c = compare::compare(
                        &rows,
                        &CompareOptions {
                            max_diff: 0.05,
                            by: Some(by.clone()),
                            ..CompareOptions::default()
                        },
                    );
                    out!("\n{}", c.text.trim_end());
                }
            }
            Ok(if outcome.failed > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
    }
}

fn check(registry: Registry, data: DataRoot, args: &CheckArgs) -> Result<ExitCode, String> {
    let ctx = context(registry, data, &args.place)?;
    let paths: Vec<PathBuf> = if args.specs.is_empty() {
        let dir = ctx.repo.join("bench").join("specs");
        let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|e| format!("reading {}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        v.sort();
        v
    } else {
        args.specs.iter().map(|s| spec_path(&ctx.repo, s)).collect()
    };
    let mut bad = 0;
    for path in &paths {
        let spec = match Spec::load(path) {
            Ok(s) => s,
            Err(e) => {
                out!("{e}");
                bad += 1;
                continue;
            }
        };
        let runs: u32 = spec
            .cases
            .iter()
            .filter(|c| c.unavailable.is_none())
            .map(|c| c.repetitions + c.warmup)
            .sum();
        out!(
            "{}: {} cases, {} runs of a command; {}",
            spec.name,
            spec.cases.len(),
            runs,
            spec.description.trim()
        );
        for c in &spec.cases {
            let data = c
                .dataset
                .as_ref()
                .map_or_else(|| "no data".into(), |d| d.label().to_string());
            let state = match &c.unavailable {
                Some(r) => format!("UNAVAILABLE: {r}"),
                None => format!(
                    "{} reps + {} warm-up, {} cache, limit {}, {}",
                    c.repetitions,
                    c.warmup,
                    format!("{:?}", c.cache).to_lowercase(),
                    human_size(c.memory_limit),
                    data
                ),
            };
            out!("  {:<48} {state}", c.id);
            if args.commands {
                let mut scope = Scope::new(spec.vars.clone());
                scope.set("repo", ctx.repo.display().to_string());
                scope.set("data", ctx.data.path().display().to_string());
                scope.set("work", ctx.work.display().to_string());
                scope.set("spec", spec.name.clone());
                scope.set(
                    "case_work",
                    ctx.work
                        .join(&spec.name)
                        .join("cases")
                        .join(path_safe(&c.id))
                        .display()
                        .to_string(),
                );
                scope.set("threads", c.threads.to_string());
                scope.set("rep", "rep1");
                scope.set("benchmark", c.benchmark.clone());
                if let Some(d) = &c.dataset {
                    scope.set("dataset", dataset_path(&ctx, d)?.display().to_string());
                }
                for (k, v) in &c.vars {
                    scope.set(k, v.clone());
                }
                let cmd = match &c.program {
                    Program::Argv(a) => a
                        .iter()
                        .map(|x| scope.expand(x))
                        .collect::<Result<Vec<_>, _>>()?
                        .join(" "),
                    Program::Shell(s) => format!("sh -c {:?}", scope.expand(s)?),
                };
                out!("      {cmd}");
            }
        }
    }
    if bad > 0 {
        return Err(format!("{bad} of {} specs failed the check", paths.len()));
    }
    Ok(ExitCode::SUCCESS)
}

fn report_files(args: &ReportArgs) -> Result<ExitCode, String> {
    let mut rows = Vec::new();
    for f in &args.files {
        rows.extend(row::read_rows(f)?);
    }
    if !args.only.is_empty() {
        rows.retain(|r| args.only.iter().any(|p| r.case.contains(p.as_str())));
    }
    if rows.is_empty() {
        return Err("the result files hold no rows (matching --only)".into());
    }
    if let Some(name) = &args.spec {
        let spec = Spec::load(&spec_path(&find_repo(None)?, name))?;
        let changed = report::use_spec_references(&mut rows, &spec);
        out!(
            "References from spec {} as it is now (SHA-256 {}…), not from the rows: {changed} rows judged against changed references.\n",
            spec.name,
            &spec.sha256[..12]
        );
    }
    if let Some(path) = &args.json {
        let text = serde_json::to_string_pretty(&report::chart_json(&rows, args.target))
            .map_err(|e| e.to_string())?;
        std::fs::write(path, text + "\n")
            .map_err(|e| format!("writing {}: {e}", path.display()))?;
    }
    if args.compare {
        let deltas = match &args.deltas {
            Some(p) => serde_json::from_str(
                &std::fs::read_to_string(p).map_err(|e| format!("reading {}: {e}", p.display()))?,
            )
            .map_err(|e| format!("{}: not a JSON object of case ids and δ: {e}", p.display()))?,
            None => BTreeMap::new(),
        };
        let c = compare::compare(
            &rows,
            &CompareOptions {
                max_diff: args.max_diff,
                deltas,
                by: args.by.clone(),
            },
        );
        if let Some(p) = &args.write_deltas {
            let text = serde_json::to_string_pretty(&c.calibrated).map_err(|e| e.to_string())?;
            std::fs::write(p, text + "\n").map_err(|e| format!("writing {}: {e}", p.display()))?;
        }
        out!("{}", c.text.trim_end());
        return Ok(if c.passed {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    out!("{}", report::markdown(&rows, args.target).trim_end());
    // A reference off, or one no counted repetition could check, fails the report.
    Ok(if report::verdict(&rows).passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn selftest(registry: Registry, data: DataRoot, args: &SelftestArgs) -> Result<ExitCode, String> {
    let ctx = context(registry, data, &args.place)?;
    let threads = args.threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map_or(1, |n| u32::try_from(n.get()).unwrap_or(u32::MAX))
    });
    let spec = Spec::parse(&selftest::spec_text(&ctx.exe, threads, args.seconds))?;
    let out_file = ctx.work.join("selftest").join(format!(
        "{}.jsonl",
        units::compact_timestamp(units::unix_now())
    ));
    let opts = RunOptions {
        out: Some(out_file.clone()),
        lock_path: lock::default_path(),
        ..RunOptions::default()
    };
    let mut stderr = std::io::stderr();
    run::run(&spec, &ctx, &opts, &mut stderr).map_err(|e| e.to_string())?;
    let rows = row::read_rows(&out_file)?;
    let (text, ok) = selftest::evaluate(&rows);
    out!("{}", text.trim_end());
    out!("rows: {}", out_file.display());
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn clean(registry: Registry, data: DataRoot, args: &CleanArgs) -> Result<ExitCode, String> {
    let ctx = context(registry, data, &args.place)?;
    if !ctx.work.join(run::WORK_MARKER).exists() {
        return Err(format!(
            "{} has no {} file, so it is not a work directory of everygraph-bench (or nothing \
             ran there yet): nothing deleted",
            ctx.work.display(),
            run::WORK_MARKER
        ));
    }
    let target = match &args.spec {
        Some(name) => ctx.work.join(name),
        None => ctx.work.clone(),
    };
    if !target.exists() {
        out!("{}: nothing there", target.display());
        return Ok(ExitCode::SUCCESS);
    }
    let size = cache::size_of(&target).unwrap_or(0);
    if args.dry_run {
        out!("would delete {} ({})", target.display(), human_size(size));
        return Ok(ExitCode::SUCCESS);
    }
    std::fs::remove_dir_all(&target).map_err(|e| format!("deleting {}: {e}", target.display()))?;
    out!("deleted {} ({})", target.display(), human_size(size));
    Ok(ExitCode::SUCCESS)
}

fn measure_command(args: &MeasureArgs) -> Result<ExitCode, String> {
    let interrupt = args
        .interrupt_after_ms
        .map(|ms| -> Result<Interrupt, String> {
            Ok(Interrupt {
                after: Duration::from_millis(ms),
                signal: match args.interrupt_signal.as_str() {
                    "INT" => StopSignal::Int,
                    "TERM" => StopSignal::Term,
                    other => return Err(format!("--interrupt-signal {other}: use INT or TERM")),
                },
                // Judged by `run`, not here.
                expect_exit: 0,
            })
        })
        .transpose()?;
    let opts = MeasureOptions {
        argv: args.command.clone(),
        sample_interval: Duration::from_millis(args.sample_interval_ms.max(1)),
        timeout: Duration::from_millis(args.timeout_ms),
        interrupt,
    };
    let m = measure(&Host::default(), &opts)?;
    let text = serde_json::to_string(&m).map_err(|e| e.to_string())?;
    let tmp = args.report.with_extension("tmp");
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, &args.report))
        .map_err(|e| format!("writing {}: {e}", args.report.display()))?;
    Ok(ExitCode::SUCCESS)
}
