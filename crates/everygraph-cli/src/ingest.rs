// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph ingest FILE` (`docs/api-sketch.md` § 5): a text edge list to `NAME.eg`,
//! with the summary, the plan, progress lines and the result block of
//! `docs/ux-contract.md` §§ 3–4, and its errors (§ 6.1).
//!
//! The graph is written as `NAME.eg.partial`, committed, read back and checked, then
//! renamed (C-1): a stop at any moment leaves no `NAME.eg` that is not whole.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use everygraph_core::{Budget, ByteSource, InsufficientBudget, Memory, MemoryProbe};
use everygraph_format::container::{Container, Input, Provenance};
use everygraph_ingest::build::{
    self, BuildError, BuildOptions, DirectedFrom, Duplicates, GraphOptions, Phase, Progress,
    Regime, Report, SelfLoops,
};
use everygraph_ingest::ids::KeyKind;
use everygraph_ingest::text::{Column, Compression, IdType, TextError, TextOptions, TextReader};
use everygraph_io::{FileSink, FileSource, MmapSource, ProcMemoryProbe};

use crate::failure::{Failure, Left};
use crate::render::{Style, about, count, duration, rough_size, size, to_f64};
use crate::scratch::JobScratch;

/// Text read per second and per thread by the build, for the plan's time estimate. A
/// placeholder of the cost model (#45 calibrates it from harness runs): about what an
/// exploratory run on lj gave at 8 threads on a loaded machine.
const MODEL_BYTES_PER_SECOND_PER_THREAD: f64 = 35e6;
/// Output bytes per adjacency entry, for the plan's result estimate (placeholder; lj and
/// orkut store about 2.1–2.3, `docs/spec/eg-adjacency.md` § 7).
const MODEL_RESULT_BYTES_PER_ENTRY: f64 = 2.5;
/// Bytes per item while its bucket waits (`everygraph_ingest::build`).
const ITEM_BYTES: u64 = 6;
/// A compressed edge list holds about this many times its size in text (placeholder).
const MODEL_COMPRESSION_RATIO: u64 = 4;
/// An operation estimated to take at least this long prints its plan (P-3; placeholder).
const LONG_SECONDS: f64 = 5.0;

/// What was asked.
#[derive(Debug, Default)]
struct Args {
    input: PathBuf,
    source: Option<String>,
    target: Option<String>,
    directed: Option<bool>,
    self_loops: SelfLoops,
    duplicates: Duplicates,
    ids: IdType,
    out: Option<PathBuf>,
    memory: Option<(u64, String)>,
    threads: Option<NonZeroUsize>,
    temp_dir: Option<PathBuf>,
    dry_run: bool,
    json: bool,
    overwrite: bool,
}

/// Runs the command; returns the exit status.
pub(crate) fn main(args: &[OsString]) -> u8 {
    let json = args.iter().any(|a| a == "--json");
    let outcome = parse(args).and_then(|args| run(&args));
    match outcome {
        Ok(()) => 0,
        Err(failure) => failure.report(json),
    }
}

fn usage(what: String) -> Failure {
    Failure::new("UsageError", what, 2).ways(vec![(
        "everygraph ingest --help".into(),
        "the options of ingest".into(),
    )])
}

fn parse(raw: &[OsString]) -> Result<Args, Failure> {
    let mut args = Args::default();
    let mut input = None;
    let mut it = raw.iter();
    while let Some(arg) = it.next() {
        let arg = arg
            .to_str()
            .ok_or_else(|| usage("An argument is not valid text.".into()))?;
        let mut value = |name: &str| -> Result<String, Failure> {
            it.next()
                .and_then(|v| v.to_str())
                .map(str::to_owned)
                .ok_or_else(|| usage(format!("{name} needs a value.")))
        };
        match arg {
            "--source" => args.source = Some(value(arg)?),
            "--target" => args.target = Some(value(arg)?),
            "--directed" => args.directed = Some(true),
            "--undirected" => args.directed = Some(false),
            "--self-loops" => {
                args.self_loops = match value(arg)?.as_str() {
                    "drop" => SelfLoops::Drop,
                    "keep" => SelfLoops::Keep,
                    other => {
                        return Err(usage(format!(
                            "--self-loops takes drop or keep, not {other}."
                        )));
                    }
                }
            }
            "--duplicates" => {
                args.duplicates = match value(arg)?.as_str() {
                    "events" => Duplicates::Events,
                    "collapse" => Duplicates::Collapse,
                    "sum" => Duplicates::Sum,
                    other => {
                        return Err(usage(format!(
                            "--duplicates takes events, collapse or sum, not {other}."
                        )));
                    }
                }
            }
            "--ids" => {
                args.ids = match value(arg)?.as_str() {
                    "auto" => IdType::Auto,
                    "string" => IdType::String,
                    other => {
                        return Err(usage(format!("--ids takes auto or string, not {other}.")));
                    }
                }
            }
            "--out" | "-o" => args.out = Some(PathBuf::from(value(arg)?)),
            "--memory" => {
                let text = value(arg)?;
                let bytes = parse_size(&text)
                    .ok_or_else(|| usage(format!("--memory {text} is not a size, such as 8GB.")))?;
                args.memory = Some((bytes, text));
            }
            "--threads" => {
                let text = value(arg)?;
                args.threads =
                    Some(text.parse().map_err(|_| {
                        usage(format!("--threads {text} is not a positive number."))
                    })?);
            }
            "--temp-dir" => args.temp_dir = Some(PathBuf::from(value(arg)?)),
            "--dry-run" => args.dry_run = true,
            "--json" => args.json = true,
            "--overwrite" => args.overwrite = true,
            "--weight" | "--time" => {
                return Err(
                    Failure::new("Unsupported", format!("{arg} cannot be stored yet."), 2)
                        .why(vec![
                            "Weight and time columns come with property columns (issue #37)."
                                .into(),
                        ])
                        .ways(vec![(
                            format!("leave out {arg}"),
                            "the edges are read without it".into(),
                        )]),
                );
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            option if option.starts_with('-') => {
                return Err(usage(format!("Unknown option {option}.")));
            }
            path if input.is_none() => input = Some(PathBuf::from(path)),
            extra => {
                return Err(usage(format!(
                    "ingest takes one FILE; {extra} is one more."
                )));
            }
        }
    }
    args.input = input.ok_or_else(|| usage("ingest needs a FILE of edges.".into()))?;
    Ok(args)
}

fn print_help() {
    let text = "\
Read a file of edges and build a graph (.eg) beside it.

Usage
  everygraph ingest FILE [OPTIONS]

Options
  --source NAME         the source column (default: from, source, or the first)
  --target NAME         the target column (default: to, target, or the second)
  --directed            the edges have a direction
  --undirected          they do not (default unless the columns say from/to)
  --self-loops drop|keep    edges from a node to itself (default: drop)
  --duplicates events|collapse|sum
                        repeated edges (default: events)
  --ids auto|string     how identifiers are read (default: auto)
  --out FILE            where the graph goes (default: NAME.eg beside FILE)
  --memory SIZE         the memory budget, for example 8GB
  --threads N           the most threads to use
  --temp-dir DIR        where scratch files go (default: beside the output)
  --dry-run             print the plan and the estimate, then stop
  --json                print the result as JSON, with nothing around it
  --overwrite           replace a file that is in the way
";
    let _ = io::stdout().write_all(text.as_bytes());
}

/// `8GB`, `512MB`, `1.5GB`, `6G`, `256MiB`, or bytes: powers of 10 for `kB`, `MB`,
/// `GB`, `TB` (§ 2.6) and of 2 for the `i` forms.
fn parse_size(text: &str) -> Option<u64> {
    let at = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(at);
    let number: f64 = number.parse().ok()?;
    let scale: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "k" | "kb" => 1e3,
        "m" | "mb" => 1e6,
        "g" | "gb" => 1e9,
        "t" | "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    let bytes = (number * scale).round();
    // Finite, non-negative and below 2^63: exact enough for a budget.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked range"
    )]
    (bytes.is_finite() && (0.0..9.2e18).contains(&bytes)).then_some(bytes as u64)
}

/// `NAME.eg` beside the input, `NAME` being its name without the extensions an edge
/// list may have (`transfers.csv.gz` → `transfers.eg`).
fn default_output(input: &Path) -> PathBuf {
    const EXTENSIONS: [&str; 9] = [
        "gz", "zst", "txt", "csv", "tsv", "edges", "el", "edgelist", "ssv",
    ];
    let mut name = input
        .file_name()
        .map_or_else(|| "graph".to_owned(), |n| n.to_string_lossy().into_owned());
    while let Some((stem, ext)) = name.rsplit_once('.') {
        if stem.is_empty() || !EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            break;
        }
        name = stem.to_owned();
    }
    input.with_file_name(format!("{name}.eg"))
}

/// What the start of the input says about its rows.
struct Sample {
    rows: u64,
    exact: bool,
    /// Rows of the largest block, for the memory of blocks in flight.
    rows_per_block: u64,
}

fn sample(reader: &TextReader<FileSource>) -> Result<Sample, TextError> {
    let bytes = reader.source().len();
    match reader.blocks()? {
        Some(0) => Ok(Sample {
            rows: 0,
            exact: true,
            rows_per_block: 0,
        }),
        Some(blocks) => {
            let (batch, _) = reader.read_block(0)?;
            let rows = u64::try_from(batch.len()).unwrap_or(u64::MAX);
            let block = u64::try_from(reader.block_bytes()).unwrap_or(u64::MAX);
            Ok(Sample {
                rows: if blocks == 1 {
                    rows
                } else {
                    rows * bytes.div_ceil(block.min(bytes))
                },
                exact: blocks == 1,
                rows_per_block: rows,
            })
        }
        None => {
            let text = bytes * MODEL_COMPRESSION_RATIO;
            Ok(Sample {
                rows: text / 15,
                exact: false,
                rows_per_block: u64::try_from(reader.block_bytes()).unwrap_or(u64::MAX) / 15,
            })
        }
    }
}

/// The plan of `docs/ux-contract.md` § 3, as far as the build can tell before it reads.
struct Plan {
    budget: u64,
    budget_origin: String,
    free: Option<u64>,
    machine: Option<u64>,
    threads: NonZeroUsize,
    working: u64,
    regime: Regime,
    seconds: f64,
    peak: u64,
    scratch: u64,
    result: u64,
    smallest: u64,
}

fn plan(args: &Args, sample: &Sample, text_bytes: u64, block_bytes: u64) -> Result<Plan, Failure> {
    let system = ProcMemoryProbe::new().probe().ok();
    let threads = args
        .threads
        .or_else(|| std::thread::available_parallelism().ok())
        .unwrap_or(NonZeroUsize::MIN);
    let free = system.map(|s| s.available());
    let (budget, budget_origin) = match &args.memory {
        Some((bytes, _)) => (*bytes, "--memory".to_owned()),
        None => (
            free.map_or(8_000_000_000, |f| f / 2),
            "default: half of free RAM".to_owned(),
        ),
    };
    if let (Some(system), Some((bytes, text))) = (system, &args.memory) {
        if *bytes > system.total() {
            return Err(Failure::new(
                "UsageError",
                "The memory budget is more than this machine has.".into(),
                2,
            )
            .why(vec![format!(
                "--memory {text} asks for {}; this machine has {} of memory in all.",
                size(*bytes),
                rough_size(system.total())
            )])
            .ways(vec![
                (
                    format!("--memory {}", typed(system.available())),
                    "what is free now".into(),
                ),
                (
                    "leave out --memory".into(),
                    format!(
                        "the default, half of free RAM: {} now",
                        size(system.available() / 2)
                    ),
                ),
            ])
            .left(Left::Nothing));
        }
    }
    let per_row = if args.directed == Some(true) { 1 } else { 2 };
    let working = sample.rows * per_row * ITEM_BYTES;
    let in_memory = working <= budget / 4 * 3;
    let seconds = to_f64(text_bytes)
        / (MODEL_BYTES_PER_SECOND_PER_THREAD
            * to_f64(threads.get().try_into().unwrap_or(u64::MAX)));
    // Blocks in flight, the dictionary's smallest working set and some headroom.
    let flight = 2 * block_bytes + sample.rows_per_block * 24;
    let smallest = u64::try_from(threads.get()).unwrap_or(u64::MAX) * flight + (24 << 20);
    Ok(Plan {
        budget,
        budget_origin,
        free,
        machine: system.map(|s| s.total()),
        threads,
        working,
        regime: if in_memory {
            Regime::InMemory
        } else {
            Regime::OnDisk
        },
        seconds,
        peak: working.min(budget),
        scratch: if in_memory {
            0
        } else {
            working.saturating_sub(budget / 2)
        },
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "an estimate"
        )]
        result: (to_f64(sample.rows * per_row) * MODEL_RESULT_BYTES_PER_ENTRY) as u64,
        smallest,
    })
}

/// A budget as it can be typed: rounded up to 100 MB below 1 GB, 0.5 GB from 1 GB,
/// 1 GB from 4 GB (P-14).
fn typed(bytes: u64) -> String {
    let step = match bytes {
        b if b < 1_000_000_000 => 100_000_000,
        b if b < 4_000_000_000 => 500_000_000,
        _ => 1_000_000_000,
    };
    let up = bytes.div_ceil(step) * step;
    if up % 1_000_000_000 == 0 {
        format!("{}GB", up / 1_000_000_000)
    } else if up >= 1_000_000_000 {
        format!("{:.1}GB", to_f64(up) / 1e9)
    } else {
        format!("{}MB", up / 1_000_000)
    }
}

/// Lines of the summary and the plan.
fn plan_text(
    args: &Args,
    reader: &TextReader<FileSource>,
    sample: &Sample,
    plan: &Plan,
    directed: (bool, DirectedFrom),
    style: Style,
    long: bool,
) -> String {
    let dot = style.dot();
    let layout = reader.layout();
    let name = args
        .input
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let rows = if sample.exact {
        format!("{} rows", count(sample.rows))
    } else {
        format!("~{} rows", rough_count(sample.rows))
    };
    let mut out = format!(
        "{}{dot}{}{dot}{rows}\n\n",
        style.bold(&name),
        size(reader.source().len())
    );
    let ids = match layout.ids {
        IdType::String => "text",
        _ => "integers",
    };
    let _ = writeln!(out, "  {}       {ids}", style.dim("ids"));
    let direction = match directed {
        (true, DirectedFrom::ColumnNames) => "directed (from the column names)",
        (false, DirectedFrom::Default) => "undirected (the column names give no direction)",
        (true, _) => "directed",
        (false, _) => "undirected",
    };
    let duplicates = match args.duplicates {
        Duplicates::Events => "duplicates kept as events",
        Duplicates::Collapse => "duplicates collapsed",
        Duplicates::Sum => "duplicates summed",
    };
    let loops = match args.self_loops {
        SelfLoops::Drop => "self-loops dropped",
        SelfLoops::Keep => "self-loops kept",
    };
    let value = format!("{direction}{dot}{duplicates}{dot}{loops}");
    if 12 + value.chars().count() <= 80 {
        let _ = writeln!(out, "  {}     {value}", style.dim("edges"));
    } else {
        // A second line, aligned with the first value, when one does not fit.
        let _ = writeln!(out, "  {}     {direction}", style.dim("edges"));
        let _ = writeln!(out, "            {duplicates}{dot}{loops}");
    }
    let arrow = style.arrow();
    let _ = writeln!(
        out,
        "  {}   {} {arrow} source{dot}{} {arrow} target",
        style.dim("columns"),
        layout.column_name(layout.source),
        layout.column_name(layout.target)
    );
    if long {
        out.push('\n');
        let threads = plan.threads.get();
        let memory = match (plan.machine, plan.free) {
            (Some(total), Some(free)) => {
                format!("{} RAM ({} free)", rough_size(total), rough_size(free))
            }
            _ => "memory unknown".into(),
        };
        let _ = writeln!(
            out,
            "  {}   {threads} threads{dot}{memory}",
            style.dim("machine")
        );
        let _ = writeln!(
            out,
            "  {}    {} memory ({})",
            style.dim("budget"),
            size(plan.budget),
            plan.budget_origin
        );
        if plan.free.is_some_and(|f| plan.budget > f) {
            let _ = writeln!(
                out,
                "            more than the {} free now: the system may swap",
                rough_size(plan.free.unwrap_or(0))
            );
        }
        let ratio = to_f64(plan.working) / to_f64(plan.budget.max(1));
        let (regime, verb) = match plan.regime {
            Regime::InMemory => ("build in memory", "fits"),
            Regime::OnDisk => ("build on disk", "is"),
        };
        let reason = if verb == "fits" {
            format!(
                "the graph fits the memory budget ({ratio:.1}{})",
                style.times()
            )
        } else {
            format!("the graph is {ratio:.1}{} the memory budget", style.times())
        };
        let _ = writeln!(
            out,
            "  {}      {regime} {} {reason}",
            style.dim("plan"),
            style.dash()
        );
        let scratch = if plan.scratch == 0 {
            "no scratch".to_owned()
        } else {
            format!("{} scratch", rough_size(plan.scratch))
        };
        let _ = writeln!(
            out,
            "  {}  {}{dot}peak {}{dot}{scratch}{dot}result {}",
            style.dim("estimate"),
            about(plan.seconds),
            rough_size(plan.peak),
            rough_size(plan.result)
        );
    }
    out
}

fn rough_count(n: u64) -> String {
    let (value, suffix) = match n {
        n if n >= 1_000_000_000 => (to_f64(n) / 1e9, " B"),
        n if n >= 1_000_000 => (to_f64(n) / 1e6, " M"),
        n if n >= 1_000 => (to_f64(n) / 1e3, "k"),
        n => return n.to_string(),
    };
    let digits = if value < 10.0 {
        2
    } else if value < 100.0 {
        1
    } else {
        0
    };
    format!("{value:.digits$}{suffix}")
}

/// The phase lines, printed as each phase ends.
struct Lines {
    style: Style,
    started: Mutex<Option<(Phase, Instant)>>,
}

impl Lines {
    fn end_phase(&self, phase: Phase, detail: String) {
        let began = self
            .started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let seconds = began
            .filter(|b| b.0 == phase)
            .map_or(0.0, |b| b.1.elapsed().as_secs_f64());
        let line = self.style.phase_done(phase.name(), &detail, seconds);
        let _ = writeln!(io::stderr(), "{line}");
    }
}

impl Progress for Lines {
    fn begin(&self, phase: Phase, _total: Option<u64>) {
        *self.started.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((phase, Instant::now()));
    }

    fn end(&self, phase: Phase, done: u64) {
        let detail = match phase {
            Phase::Reading => format!("{} rows", count(done)),
            Phase::Identifiers => format!("{} identifiers", count(done)),
            Phase::Building if done == 1 => "1 part".to_owned(),
            Phase::Building => format!("{} parts", count(done)),
            Phase::Checking => format!("{} sections verified", count(done)),
        };
        self.end_phase(phase, detail);
    }
}

/// Removes `NAME.eg.partial` unless the graph was renamed into place.
struct Partial(Option<PathBuf>);

impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn io_failure(verb: &str, path: &Path, e: &io::Error) -> Failure {
    Failure::new(
        "IoError",
        format!("Cannot {verb} {}: {}.", path.display(), plain(e)),
        1,
    )
}

/// The operating system's reason, without the `(os error N)` it appends.
fn plain(e: &io::Error) -> String {
    let text = e.to_string();
    text.split(" (os error").next().unwrap_or(&text).to_owned()
}

fn text_failure(input: &Path, e: TextError) -> Failure {
    let file = input.display().to_string();
    match e {
        TextError::Read(cause) => {
            Failure::new("IoError", format!("Cannot read {file}: {cause}."), 1)
        }
        TextError::InvalidRows(summary) => {
            let mut why = Vec::new();
            for p in &summary.problems {
                why.push(format!("{file}:{:<12}{p}", p.line));
                why.push(format!("  {}", String::from_utf8_lossy(&p.excerpt)));
            }
            if summary.invalid_rows > u64::try_from(summary.problems.len()).unwrap_or(0) {
                why.push(format!(
                    "and {} more",
                    count(
                        summary.invalid_rows - u64::try_from(summary.problems.len()).unwrap_or(0)
                    )
                ));
            }
            why.push(format!(
                "The whole file was checked: {} lines, {} of them invalid.",
                count(summary.lines),
                count(summary.invalid_rows)
            ));
            let n = summary.invalid_rows;
            let rows = if n == 1 { "line" } else { "lines" };
            Failure::new(
                "InvalidInput",
                format!("Cannot read {file}: {}.", TextError::InvalidRows(summary)),
                3,
            )
            .why(why)
            .ways(vec![(
                format!("correct those {n} {rows}, then run the same command again"),
                String::new(),
            )])
        }
        other @ (TextError::UnknownColumn { .. }
        | TextError::NoHeader { .. }
        | TextError::BeyondHeader { .. }) => {
            Failure::new("UsageError", format!("Cannot read {file}: {other}."), 2)
        }
        other => Failure::new("InvalidInput", format!("Cannot read {file}: {other}."), 3),
    }
}

fn build_failure(input: &Path, plan: &Plan, e: BuildError) -> Failure {
    match e {
        BuildError::Text(text) => text_failure(input, text),
        BuildError::Budget(refused) => budget_failure(plan, &refused),
        BuildError::InvalidKeys(keys) => {
            let mut why: Vec<String> = keys
                .first
                .iter()
                .map(|k| format!("{} does not fit the kind of the other identifiers", k.key))
                .collect();
            why.push(format!("{} identifiers in all.", count(keys.count)));
            Failure::new(
                "InvalidInput",
                format!(
                    "Cannot read {}: some identifiers do not fit their column.",
                    input.display()
                ),
                3,
            )
            .why(why)
            .ways(vec![(
                "--ids string".into(),
                "read every identifier as text".into(),
            )])
        }
        BuildError::Scratch(e) => Failure::new(
            "IoError",
            format!("Cannot write the scratch files: {}.", plain(&e)),
            1,
        ),
        BuildError::InputChanged => Failure::new(
            "InvalidInput",
            format!("{} changed while it was read.", input.display()),
            3,
        )
        .ways(vec![(
            "run the same command again once the file is complete".into(),
            String::new(),
        )]),
        other => Failure::new("EverygraphError", format!("The build failed: {other}."), 1),
    }
}

fn budget_failure(plan: &Plan, refused: &InsufficientBudget) -> Failure {
    let smallest = plan
        .budget
        .saturating_sub(refused.available)
        .saturating_add(refused.needed);
    let mut ways = Vec::new();
    match plan.machine {
        Some(machine) if smallest > machine => {
            ways.push((
                format!("a machine with at least {} of memory", rough_size(smallest)),
                String::new(),
            ));
        }
        _ => ways.push((
            format!("--memory {}", typed(smallest)),
            "a budget that holds this step".into(),
        )),
    }
    Failure::new(
        "InsufficientBudget",
        "Not enough memory for this step.".into(),
        4,
    )
    .why(vec![
        format!(
            "The build needs {} for {}.",
            size(refused.needed),
            refused.purpose
        ),
        format!(
            "The budget is {}, and {} of it was left.",
            size(plan.budget),
            size(refused.available)
        ),
    ])
    .ways(ways)
    .bytes(refused.needed, plan.budget)
}

fn run(args: &Args) -> Result<(), Failure> {
    let started = Instant::now();
    let started_at = now_ns();
    let err = Style::of(true);
    let out_style = Style::of(false);
    let source = FileSource::open(&args.input).map_err(|e| io_failure("read", &args.input, &e))?;
    let options = TextOptions {
        source: args.source.as_deref().map(Column::from),
        target: args.target.as_deref().map(Column::from),
        ids: args.ids,
        ..TextOptions::default()
    };
    let reader = TextReader::open(source, &options).map_err(|e| text_failure(&args.input, e))?;
    let directed = build::infer_direction(reader.layout(), args.directed);
    let output = args
        .out
        .clone()
        .unwrap_or_else(|| default_output(&args.input));
    if output.exists() && !args.overwrite {
        let ours = File::open(&output)
            .ok()
            .and_then(|f| FileSource::from_file(f).ok())
            .is_some_and(|s| Container::open(s).is_ok());
        let holds = if ours {
            "holds another graph"
        } else {
            "was not written by Everygraph"
        };
        return Err(Failure::new(
            "OutputExists",
            format!("{} exists and {holds}.", output.display()),
            1,
        )
        .ways(vec![
            (
                "--out NEW.eg".into(),
                "write the graph under another name".into(),
            ),
            ("--overwrite".into(), "replace it".into()),
        ])
        .left(Left::Nothing));
    }
    let sample = sample(&reader).map_err(|e| text_failure(&args.input, e))?;
    let text_bytes = match reader.layout().compression {
        Compression::None => reader.source().len(),
        _ => reader.source().len() * MODEL_COMPRESSION_RATIO,
    };
    let block_bytes = u64::try_from(reader.block_bytes()).unwrap_or(u64::MAX);
    let plan = plan(args, &sample, text_bytes, block_bytes)?;
    let long = args.dry_run || plan.seconds >= LONG_SECONDS;
    let fits = plan.budget >= plan.smallest;
    if args.dry_run {
        let mut text = plan_text(args, &reader, &sample, &plan, directed, out_style, true);
        let arrow = out_style.arrow();
        let _ = write!(
            text,
            "\n  {}    reading {arrow} identifiers {arrow} building {arrow} checking\n  {}    {}\n\nDry run: nothing was written.\n  to run it: the same command without --dry-run\n",
            out_style.dim("phases"),
            out_style.dim("result"),
            output.display()
        );
        if !args.json {
            let _ = io::stdout().write_all(text.as_bytes());
        } else {
            let _ = io::stderr().write_all(text.as_bytes());
        }
        if !fits {
            return Err(refusal(&plan));
        }
        if args.json {
            let _ = writeln!(
                io::stdout(),
                "{{\"schema\":1,\"operation\":\"ingest\",\"fits\":true}}"
            );
        }
        return Ok(());
    }
    let _ = io::stderr()
        .write_all(plan_text(args, &reader, &sample, &plan, directed, err, long).as_bytes());
    let _ = writeln!(io::stderr());
    if !fits {
        return Err(refusal(&plan));
    }

    // Threads are the binary's to decide; the library only reserves for them.
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(plan.threads.get())
        .build_global();
    let root = match &args.temp_dir {
        Some(dir) => dir.clone(),
        None => output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
    };
    let job = Job {
        id: job_id(args, &output, &reader),
        args,
        plan: &plan,
        output: &output,
        started,
        started_at,
        style: err,
        graph: GraphOptions {
            directed: directed.0,
            self_loops: args.self_loops,
            duplicates: args.duplicates,
        },
    };
    let scratch = JobScratch::create(&root, &job.id).map_err(|e| io_failure("write", &root, &e))?;
    let lines = Lines {
        style: err,
        started: Mutex::new(None),
    };
    if reader
        .blocks()
        .map_err(|e| text_failure(&args.input, e))?
        .is_some()
    {
        return job.write(&reader, &scratch, &lines);
    }
    // A compressed input is read once into a plain copy in the scratch directory.
    lines.begin(Phase::Reading, None);
    let path = scratch.dir().join("input.txt");
    let file = File::create(&path).map_err(|e| io_failure("write", &path, &e))?;
    let mut writer = BufWriter::with_capacity(1 << 20, file);
    reader
        .decompress_into(&mut writer)
        .map_err(|e| text_failure(&args.input, e))?;
    writer
        .into_inner()
        .map_err(|e| io_failure("write", &path, e.error()))?;
    let copy = MmapSource::open(&path).map_err(|e| io_failure("read", &path, &e))?;
    let _ = std::fs::remove_file(&path);
    let plain = TextReader::open(copy, &options).map_err(|e| text_failure(&args.input, e))?;
    job.write(&plain, &scratch, &lines)
}

/// What a run needs once its plan fits.
struct Job<'a> {
    id: String,
    args: &'a Args,
    plan: &'a Plan,
    output: &'a Path,
    started: Instant,
    started_at: Option<i64>,
    style: Style,
    graph: GraphOptions,
}

impl Job<'_> {
    /// Builds the graph from `reader` into `NAME.eg.partial`, checks it, and renames it.
    fn write<S: ByteSource + Sync>(
        &self,
        reader: &TextReader<S>,
        scratch: &JobScratch,
        lines: &Lines,
    ) -> Result<(), Failure> {
        let (args, plan, output) = (self.args, self.plan, self.output);
        let memory = Memory::from(Arc::new(Budget::new(plan.budget)));
        let mut options = BuildOptions::new(self.graph, plan.threads);
        if args.ids == IdType::String {
            options.ids = options.ids.with_kind(KeyKind::Bytes);
        }
        let partial_path = PathBuf::from(format!("{}.partial", output.display()));
        let sink =
            FileSink::create(&partial_path).map_err(|e| io_failure("write", &partial_path, &e))?;
        let mut partial = Partial(Some(partial_path.clone()));
        let unfinished = build::build(reader, &options, &memory, scratch, sink, lines)
            .map_err(|e| build_failure(&args.input, plan, e))?;
        let metadata = std::fs::metadata(&args.input).ok();
        let input = Input {
            name: args.input.display().to_string(),
            bytes: metadata.as_ref().map_or(0, std::fs::Metadata::len),
            modified: metadata
                .and_then(|m| m.modified().ok())
                .and_then(ns_since_epoch),
            ..Input::default()
        };
        let provenance = Provenance {
            engine: format!("everygraph {}", env!("CARGO_PKG_VERSION")),
            started: self.started_at,
            finished: now_ns(),
            inputs: vec![input],
            ..Provenance::default()
        };
        let (sink, report) = unfinished
            .commit(provenance)
            .map_err(|e| build_failure(&args.input, plan, e))?;
        drop(sink.into_file());

        lines.begin(Phase::Checking, None);
        let written =
            MmapSource::open(&partial_path).map_err(|e| io_failure("read", &partial_path, &e))?;
        let checked = build::check(written, &memory).map_err(|e| {
            Failure::new(
                "CorruptFile",
                format!("{} is damaged: {e}.", partial_path.display()),
                5,
            )
        })?;
        lines.end(
            Phase::Checking,
            u64::try_from(checked.sections).unwrap_or(u64::MAX),
        );
        everygraph_io::persist(&partial_path, output)
            .map_err(|e| io_failure("write", output, &e))?;
        partial.0 = None;

        let bytes = std::fs::metadata(output).map_or(0, |m| m.len());
        let elapsed = self.started.elapsed().as_secs_f64();
        result_block(output, bytes, &report, elapsed, self.style);
        if args.json {
            let document = file_document(&self.id, output, bytes, &report, elapsed);
            let _ = writeln!(io::stdout(), "{document}");
        }
        Ok(())
    }
}

fn refusal(plan: &Plan) -> Failure {
    let refused = InsufficientBudget::new("text blocks in flight", plan.smallest, plan.budget);
    let threads = plan.threads.get();
    let mut failure = budget_failure(plan, &refused);
    if threads > 1 {
        failure = failure.more_way(
            "--threads 1".into(),
            "fewer blocks in flight need less memory".into(),
        );
    }
    failure.why(vec![format!(
        "{threads} threads read blocks at once; they need about {} and the budget is {}.",
        rough_size(plan.smallest),
        size(plan.budget)
    )])
}

fn result_block(output: &Path, bytes: u64, report: &Report, seconds: f64, style: Style) {
    let dot = style.dot();
    let name = output
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut text = format!(
        "\n{} {}{dot}{}{dot}{} nodes{dot}{} edges{dot}{}\n",
        style.ok(),
        style.bold(&name),
        size(bytes),
        count(report.nodes),
        count(report.edges),
        duration(seconds)
    );
    let mut dropped = Vec::new();
    if report.dropped_self_loops > 0 {
        dropped.push(format!("{} self-loops", count(report.dropped_self_loops)));
    }
    if report.dropped_duplicates > 0 {
        dropped.push(format!("{} duplicates", count(report.dropped_duplicates)));
    }
    if report.rejected > 0 {
        dropped.push(format!("{} rejected rows", count(report.rejected)));
    }
    if !dropped.is_empty() {
        let _ = writeln!(text, "  dropped: {}", dropped.join(dot));
    }
    if report.merged > 0 {
        let _ = writeln!(
            text,
            "  merged: {} duplicates, summed into their pairs",
            count(report.merged)
        );
    }
    let _ = writeln!(
        text,
        "  next: {}",
        style.bold(&format!("everygraph map {}", output.display()))
    );
    let _ = io::stderr().write_all(text.as_bytes());
}

/// The file document of `docs/ux-contract.md` § 4.7.
fn file_document(job: &str, output: &Path, bytes: u64, report: &Report, seconds: f64) -> String {
    let counts: Vec<String> = report
        .counts()
        .iter()
        .filter_map(|(k, v)| k.strip_prefix("counts.").map(|k| format!("\"{k}\":{v}")))
        .collect();
    let dropped: Vec<String> = report
        .counts()
        .iter()
        .filter(|(_, v)| *v > 0)
        .filter_map(|(k, v)| k.strip_prefix("dropped.").map(|k| format!("\"{k}\":{v}")))
        .collect();
    format!(
        "{{\"schema\":1,\"job\":\"{job}\",\"operation\":\"ingest\",\"made\":{{\"path\":{},\"kind\":\"graph\",\"bytes\":{bytes}}},\"counts\":{{{}}},\"dropped\":{{{}}},\"kept\":[],\"elapsed_seconds\":{seconds:.3}}}",
        crate::failure::json_string(&output.display().to_string()),
        counts.join(","),
        dropped.join(",")
    )
}

/// The first 16 hexadecimal digits of the job's fingerprint: the input, the output and
/// every option that changes the graph (not the budget or the threads; R-4).
fn job_id(args: &Args, output: &Path, reader: &TextReader<FileSource>) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::fs::canonicalize(&args.input)
        .unwrap_or_else(|_| args.input.clone())
        .hash(&mut h);
    output.hash(&mut h);
    reader.source().len().hash(&mut h);
    format!(
        "{:?}{:?}{:?}{:?}{:?}{:?}",
        args.source, args.target, args.directed, args.self_loops, args.duplicates, args.ids
    )
    .hash(&mut h);
    format!("{:016x}", h.finish())
}

fn now_ns() -> Option<i64> {
    ns_since_epoch(SystemTime::now())
}

fn ns_since_epoch(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_read_as_the_contract_writes_them() {
        assert_eq!(parse_size("8GB"), Some(8_000_000_000));
        assert_eq!(parse_size("1.5GB"), Some(1_500_000_000));
        assert_eq!(parse_size("512MB"), Some(512_000_000));
        assert_eq!(parse_size("256MiB"), Some(256 << 20));
        assert_eq!(parse_size("6G"), Some(6_000_000_000));
        assert_eq!(parse_size("100"), Some(100));
        assert_eq!(parse_size("GB"), None);
        assert_eq!(parse_size("8 parsecs"), None);
    }

    #[test]
    fn budgets_are_rounded_up_to_a_value_that_can_be_typed() {
        assert_eq!(typed(5_598_053_768), "6GB");
        assert_eq!(typed(1_200_000_000), "1.5GB");
        assert_eq!(typed(230_000_000), "300MB");
        assert_eq!(typed(4_000_000_000), "4GB");
    }

    #[test]
    fn the_output_drops_the_extensions_of_an_edge_list() {
        assert_eq!(
            default_output(Path::new("a/transfers.csv.gz")),
            Path::new("a/transfers.eg")
        );
        assert_eq!(default_output(Path::new("lj.txt")), Path::new("lj.eg"));
        assert_eq!(
            default_output(Path::new("graph.v2.tsv")),
            Path::new("graph.v2.eg")
        );
        assert_eq!(default_output(Path::new(".csv")), Path::new(".csv.eg"));
    }
}
