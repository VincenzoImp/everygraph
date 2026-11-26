// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph ingest FILE` (`docs/api-sketch.md` § 5): a text edge list to `NAME.eg`,
//! with the summary, the plan, progress lines and the result block of
//! `docs/ux-contract.md` §§ 3–4, and its errors (§ 6.1).
//!
//! The graph is written as `NAME.eg.partial`, committed, read back and checked, then
//! renamed (C-1): a stop at any moment leaves no `NAME.eg` that is not whole.

use std::fs::File;
use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use everygraph_core::{ByteSource, par};
use everygraph_format::container::{Container, Input, Provenance};
use everygraph_ingest::build::{
    self, BuildOptions, Duplicates, GraphOptions, Phase, Progress, Report, SelfLoops,
};
use everygraph_ingest::ids::KeyKind;
use everygraph_ingest::text::{Column, IdType, TextOptions, TextReader};
use everygraph_io::FileSource;
use serde_json::{Map, Value, json};

use clap::builder::{PossibleValuesParser, TypedValueParser};

use crate::args::Common;
use crate::failure::{self, Failure};
use crate::machine;
use crate::plan::Plan;
use crate::progress::Lines;
use crate::progress::now_ns;
use crate::render::{Style, counted, duration, size};
use everygraph_ingest::native::{self, JobScratch, WriteError};

/// What was asked.
#[derive(Debug, clap::Args)]
#[command(override_help = HELP)]
pub(crate) struct Args {
    #[arg(value_name = "FILE")]
    pub(crate) input: PathBuf,
    #[arg(long, value_name = "NAME")]
    pub(crate) source: Option<String>,
    #[arg(long, value_name = "NAME")]
    pub(crate) target: Option<String>,
    #[arg(long)]
    directed: bool,
    #[arg(long, conflicts_with = "directed")]
    undirected: bool,
    #[arg(long = "self-loops", default_value = "drop", value_parser = PossibleValuesParser::new(["drop", "keep"])
        .map(|s| if s == "keep" { SelfLoops::Keep } else { SelfLoops::Drop }))]
    pub(crate) self_loops: SelfLoops,
    #[arg(long, default_value = "events", value_parser = PossibleValuesParser::new(["events", "collapse", "sum"])
        .map(|s| match s.as_str() { "collapse" => Duplicates::Collapse, "sum" => Duplicates::Sum, _ => Duplicates::Events }))]
    pub(crate) duplicates: Duplicates,
    #[arg(long, default_value = "auto", value_parser = PossibleValuesParser::new(["auto", "string"])
        .map(|s| if s == "string" { IdType::String } else { IdType::Auto }))]
    pub(crate) ids: IdType,
    #[arg(long, value_name = "DIR")]
    pub(crate) temp_dir: Option<PathBuf>,
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// A weight or time column, which this version cannot store.
    #[arg(long, hide = true, value_name = "NAME")]
    weight: Option<String>,
    #[arg(long, hide = true, value_name = "NAME")]
    time: Option<String>,
    #[command(flatten)]
    pub(crate) common: Common,
}

impl Args {
    /// `--directed`, `--undirected`, or neither.
    pub(crate) fn directed(&self) -> Option<bool> {
        match (self.directed, self.undirected) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        }
    }
}

/// Runs the command; returns the exit status.
pub(crate) fn main(args: &Args) -> u8 {
    let unsupported = [("--weight", &args.weight), ("--time", &args.time)]
        .into_iter()
        .find(|(_, v)| v.is_some());
    let outcome = match unsupported {
        Some((option, _)) => Err(Failure::new(
            "Unsupported",
            format!("{option} cannot be stored yet."),
            failure::USAGE,
        )
        .why(vec!["This version stores no weight or time column.".into()])
        .ways(vec![(
            format!("leave out {option}"),
            "the edges are read without it".into(),
        )])),
        None => run(args),
    };
    outcome.unwrap_or_else(|failure| failure.report(args.common.json))
}

const HELP: &str = "\
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
  --progress plain      progress as one line per finished phase, never redrawn
  -h, --help            this text

Short forms: -o for --out, -m for --memory, -t for --threads.
";

fn run(args: &Args) -> Result<u8, Failure> {
    let started = Instant::now();
    let started_at = now_ns();
    let source = FileSource::open(&args.input).map_err(|e| failure::io("read", &args.input, &e))?;
    let options = TextOptions {
        source: args.source.as_deref().map(Column::from),
        target: args.target.as_deref().map(Column::from),
        ids: args.ids,
        ..TextOptions::default()
    };
    let reader = TextReader::open(source, &options).map_err(|e| failure::text(&args.input, e))?;
    let directed = build::infer_direction(reader.layout(), args.directed());
    let output = args
        .common
        .out
        .clone()
        .unwrap_or_else(|| native::default_output(&args.input));
    if output.exists() && !args.common.overwrite {
        return Err(failure::output_exists(&output, is_graph(&output)));
    }
    let root = match &args.temp_dir {
        Some(dir) => dir.clone(),
        None => output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
    };
    let plan = Plan::new(args, &reader, directed, &output, &root)?;
    if args.dry_run {
        return Ok(dry_run(args, &reader, &plan, &output));
    }
    let err = Style::stderr();
    let text = plan.text(args, &reader, err, plan.long());
    let _ = writeln!(io::stderr(), "{text}");
    if !plan.fits() {
        return Err(plan.refusal());
    }

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
    let scratch =
        JobScratch::create(&root, &job.id).map_err(|e| failure::io("write", &root, &e))?;
    let lines = Lines::new(err.progress(args.common.plain()));
    let blocks = reader.blocks().map_err(|e| failure::text(&args.input, e))?;
    if blocks.is_some() {
        return in_pool(plan.threads, || job.write(&reader, &scratch, &lines));
    }
    // A compressed input is read once into a plain copy in the scratch directory.
    lines.begin(Phase::Reading, None);
    let plain = native::decompress(&reader, &scratch, &options).map_err(|e| job.failure(e))?;
    in_pool(plan.threads, || job.write(&plain, &scratch, &lines))
}

/// Whether `path` is a graph file.
pub(crate) fn is_graph(path: &Path) -> bool {
    File::open(path)
        .ok()
        .and_then(|f| FileSource::from_file(f).ok())
        .is_some_and(|s| Container::open(s).is_ok())
}

/// Prints the plan and what would happen, writes nothing, and returns the exit status
/// (§ 2.1: all of it on standard output; with `--json` the plan document there and
/// the text on standard error).
fn dry_run(args: &Args, reader: &TextReader<FileSource>, plan: &Plan, output: &Path) -> u8 {
    let style = if args.common.json {
        Style::stderr()
    } else {
        Style::stdout()
    };
    let mut text = plan.text(args, reader, style, true);
    let arrow = style.arrow();
    text.push_str(&format!(
        "\n  {}    reading {arrow} identifiers {arrow} building {arrow} checking\n  {}    {}\n\n\
         Dry run: nothing was written.\n  to run it: the same command without --dry-run\n",
        style.dim("phases"),
        style.dim("result"),
        output.display()
    ));
    let refusal = (!plan.fits()).then(|| plan.refusal());
    if args.common.json {
        let _ = io::stderr().write_all(text.as_bytes());
        let document = plan.document(args, reader, refusal.as_ref());
        let _ = writeln!(io::stdout(), "{document}");
    } else {
        let _ = io::stdout().write_all(text.as_bytes());
    }
    match refusal {
        Some(refusal) => {
            refusal.print_text();
            refusal.status()
        }
        None => 0,
    }
}

/// Runs `f` on a pool of `threads` threads: the binary decides how many exist, and the
/// build reserves its buffers for that many.
fn in_pool(
    threads: NonZeroUsize,
    f: impl FnOnce() -> Result<(), Failure> + Send,
) -> Result<u8, Failure> {
    match par::with_threads(threads, f) {
        Ok(result) => result.map(|()| 0),
        Err(e) => Err(failure::internal(&format!(
            "Cannot start the threads: {e}."
        ))),
    }
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
    fn failure(&self, e: WriteError) -> Failure {
        match e {
            WriteError::Build(e) => failure::build(
                &self.args.input,
                self.plan.budget,
                self.plan.machine_memory(),
                e,
            ),
            WriteError::Io { path, error } => failure::io("write", &path, &error),
            WriteError::Withdrawn => {
                failure::internal("The graph was withdrawn before it was named.")
            }
        }
    }

    /// Builds the graph from `reader` into `NAME.eg.partial`, checks it, and renames it.
    fn write<S: ByteSource + Sync>(
        &self,
        reader: &TextReader<S>,
        scratch: &JobScratch,
        lines: &Lines,
    ) -> Result<(), Failure> {
        let (args, plan, output) = (self.args, self.plan, self.output);
        let memory = machine::memory(plan.budget);
        let mut options = BuildOptions::new(self.graph, plan.threads);
        if args.ids == IdType::String {
            options.ids = options.ids.with_kind(KeyKind::Bytes);
        }
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
        let (written, report, _) = native::write(
            reader,
            &options,
            &memory,
            scratch,
            native::Output::File(output),
            provenance,
            lines,
            &|| true,
        )
        .map_err(|e| self.failure(e))?;
        let bytes = match written {
            native::Written::File(_, bytes) => bytes,
            native::Written::Memory(b) => b.len() as u64,
        };
        let elapsed = self.started.elapsed().as_secs_f64();
        result_block(output, bytes, &report, elapsed, self.style);
        if args.common.json {
            let document = file_document(&self.id, output, bytes, &report, elapsed);
            let _ = writeln!(io::stdout(), "{document}");
        }
        Ok(())
    }
}

/// The result block of a graph made (G-11 to G-13), on standard error.
fn result_block(output: &Path, bytes: u64, report: &Report, seconds: f64, style: Style) {
    let dot = style.dot();
    let name = output
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut text = format!(
        "\n{} {}{dot}{}{dot}{}{dot}{}{dot}{}\n",
        style.ok(),
        style.bold(&name),
        size(bytes),
        counted(report.nodes, "node", "nodes"),
        counted(report.edges, "edge", "edges"),
        duration(seconds)
    );
    let mut dropped = Vec::new();
    if report.dropped_self_loops > 0 {
        dropped.push(counted(
            report.dropped_self_loops,
            "self-loop",
            "self-loops",
        ));
    }
    if report.dropped_duplicates > 0 {
        dropped.push(counted(
            report.dropped_duplicates,
            "duplicate",
            "duplicates",
        ));
    }
    if report.rejected > 0 {
        dropped.push(counted(report.rejected, "rejected row", "rejected rows"));
    }
    if !dropped.is_empty() {
        text.push_str(&format!("  dropped: {}\n", dropped.join(dot)));
    }
    if report.merged > 0 {
        let merged = counted(report.merged, "duplicate", "duplicates");
        text.push_str(&format!("  merged: {merged}, summed into their pairs\n"));
    }
    let next = format!("everygraph map {}", output.display());
    text.push_str(&format!("  next: {}\n", style.bold(&next)));
    let _ = io::stderr().write_all(text.as_bytes());
}

/// The file document of `docs/ux-contract.md` § 4.7.
fn file_document(job: &str, output: &Path, bytes: u64, report: &Report, seconds: f64) -> Value {
    let mut counts = Map::new();
    let mut dropped = Map::new();
    for (key, value) in report.counts() {
        if let Some(key) = key.strip_prefix("counts.") {
            counts.insert(key.into(), value.into());
        } else if let Some(key) = key.strip_prefix("dropped.").filter(|_| value > 0) {
            dropped.insert(key.into(), value.into());
        }
    }
    json!({
        "schema": 1,
        "job": job,
        "operation": "ingest",
        "made": {"path": output.display().to_string(), "kind": "graph", "bytes": bytes},
        "counts": counts,
        "dropped": dropped,
        "kept": [],
        "elapsed_seconds": seconds,
    })
}

/// The first 16 hexadecimal digits of the job's fingerprint (XXH3-128): the input, the
/// output and every option that changes the graph, not the budget or the threads (R-4).
fn job_id(args: &Args, output: &Path, reader: &TextReader<FileSource>) -> String {
    let (input, output) = (native::canonical(&args.input), native::canonical(output));
    native::job_id(&[
        &input.display().to_string(),
        &output.display().to_string(),
        &reader.source().len().to_string(),
        &format!("{:?}", args.source),
        &format!("{:?}", args.target),
        &format!("{:?}", args.directed()),
        &format!("{:?}", args.self_loops),
        &format!("{:?}", args.duplicates),
        &format!("{:?}", args.ids),
    ])
}

fn ns_since_epoch(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
}
