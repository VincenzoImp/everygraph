// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph ingest FILE` (`docs/api-sketch.md` § 5): a text edge list to `NAME.eg`,
//! with the summary, the plan, progress lines and the result block of
//! `docs/ux-contract.md` §§ 3–4, and its errors (§ 6.1).
//!
//! The graph is written as `NAME.eg.partial`, committed, read back and checked, then
//! renamed (C-1): a stop at any moment leaves no `NAME.eg` that is not whole.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use everygraph_core::{Budget, ByteSource, Memory, par};
use everygraph_format::container::{Container, Input, Provenance};
use everygraph_ingest::build::{
    self, BuildOptions, Duplicates, GraphOptions, Phase, Progress, SelfLoops,
};
use everygraph_ingest::ids::KeyKind;
use everygraph_ingest::text::{Column, IdType, TextOptions, TextReader};
use everygraph_io::{FileSink, FileSource, MmapSource};

use crate::failure::{self, Failure, usage};
use crate::plan::Plan;
use crate::render::{Style, counted, file_document, parse_size, result_block};
use crate::scratch::JobScratch;

/// What was asked.
#[derive(Debug, Default)]
pub(crate) struct Args {
    pub(crate) input: PathBuf,
    pub(crate) source: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) directed: Option<bool>,
    pub(crate) self_loops: SelfLoops,
    pub(crate) duplicates: Duplicates,
    pub(crate) ids: IdType,
    pub(crate) out: Option<PathBuf>,
    pub(crate) memory: Option<(u64, String)>,
    pub(crate) threads: Option<NonZeroUsize>,
    pub(crate) temp_dir: Option<PathBuf>,
    pub(crate) dry_run: bool,
    pub(crate) json: bool,
    pub(crate) overwrite: bool,
}

/// Runs the command; returns the exit status.
pub(crate) fn main(args: &[OsString]) -> u8 {
    let json = args.iter().any(|a| a == "--json");
    match parse(args).and_then(|args| run(&args)) {
        Ok(status) => status,
        Err(failure) => failure.report(json),
    }
}

fn choice<T: Copy>(name: &str, value: &str, options: &[(&str, T)]) -> Result<T, Failure> {
    options
        .iter()
        .find(|(word, _)| *word == value)
        .map(|&(_, v)| v)
        .ok_or_else(|| {
            let words: Vec<&str> = options.iter().map(|(w, _)| *w).collect();
            usage(format!("{name} takes {}, not {value}.", words.join(" or ")))
        })
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
                let options = [("drop", SelfLoops::Drop), ("keep", SelfLoops::Keep)];
                args.self_loops = choice(arg, &value(arg)?, &options)?;
            }
            "--duplicates" => {
                let options = [
                    ("events", Duplicates::Events),
                    ("collapse", Duplicates::Collapse),
                    ("sum", Duplicates::Sum),
                ];
                args.duplicates = choice(arg, &value(arg)?, &options)?;
            }
            "--ids" => {
                let options = [("auto", IdType::Auto), ("string", IdType::String)];
                args.ids = choice(arg, &value(arg)?, &options)?;
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
                let threads = text
                    .parse()
                    .map_err(|_| usage(format!("--threads {text} is not a positive number.")))?;
                args.threads = Some(threads);
            }
            "--temp-dir" => args.temp_dir = Some(PathBuf::from(value(arg)?)),
            "--dry-run" => args.dry_run = true,
            "--json" => args.json = true,
            "--overwrite" => args.overwrite = true,
            "--weight" | "--time" => {
                return Err(
                    Failure::new("Unsupported", format!("{arg} cannot be stored yet."), 2)
                        .why(vec!["This version stores no weight or time column.".into()])
                        .ways(vec![(
                            format!("leave out {arg}"),
                            "the edges are read without it".into(),
                        )]),
                );
            }
            "-h" | "--help" => {
                let _ = io::stdout().write_all(HELP.as_bytes());
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
";

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

/// The phase lines, printed as each phase ends.
struct Lines {
    style: Style,
    started: Mutex<Option<(Phase, Instant)>>,
}

impl Progress for Lines {
    fn begin(&self, phase: Phase, _total: Option<u64>) {
        *self.started.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((phase, Instant::now()));
    }

    fn end(&self, phase: Phase, done: u64) {
        let detail = match phase {
            Phase::Reading => counted(done, "row", "rows"),
            Phase::Identifiers => counted(done, "identifier", "identifiers"),
            Phase::Building => counted(done, "part", "parts"),
            Phase::Checking => format!("{} verified", counted(done, "section", "sections")),
        };
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

/// Removes `NAME.eg.partial` unless the graph was renamed into place.
struct Partial(Option<PathBuf>);

impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

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
    let directed = build::infer_direction(reader.layout(), args.directed);
    let output = args
        .out
        .clone()
        .unwrap_or_else(|| default_output(&args.input));
    if output.exists() && !args.overwrite {
        return Err(output_exists(&output));
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
    let err = Style::of(true);
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
    let lines = Lines {
        style: err,
        started: Mutex::new(None),
    };
    let blocks = reader.blocks().map_err(|e| failure::text(&args.input, e))?;
    if blocks.is_some() {
        return in_pool(plan.threads, || job.write(&reader, &scratch, &lines));
    }
    // A compressed input is read once into a plain copy in the scratch directory.
    lines.begin(Phase::Reading, None);
    let path = scratch.dir().join("input.txt");
    let file = File::create(&path).map_err(|e| failure::io("write", &path, &e))?;
    let mut writer = BufWriter::with_capacity(1 << 20, file);
    reader
        .decompress_into(&mut writer)
        .map_err(|e| failure::text(&args.input, e))?;
    writer
        .into_inner()
        .map_err(|e| failure::io("write", &path, e.error()))?;
    let copy = MmapSource::open(&path).map_err(|e| failure::io("read", &path, &e))?;
    let _ = std::fs::remove_file(&path);
    let plain = TextReader::open(copy, &options).map_err(|e| failure::text(&args.input, e))?;
    in_pool(plan.threads, || job.write(&plain, &scratch, &lines))
}

fn output_exists(output: &Path) -> Failure {
    let ours = File::open(output)
        .ok()
        .and_then(|f| FileSource::from_file(f).ok())
        .is_some_and(|s| Container::open(s).is_ok());
    let holds = if ours {
        "holds another graph"
    } else {
        "was not written by Everygraph"
    };
    Failure::new(
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
}

/// Prints the plan and what would happen, writes nothing, and returns the exit status
/// (§ 2.1: all of it on standard output; with `--json` the plan document there and
/// the text on standard error).
fn dry_run(args: &Args, reader: &TextReader<FileSource>, plan: &Plan, output: &Path) -> u8 {
    let style = Style::of(!args.json);
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
    if args.json {
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
        Err(e) => Err(Failure::new(
            "EverygraphError",
            format!("Cannot start the threads: {e}."),
            1,
        )),
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
    /// Builds the graph from `reader` into `NAME.eg.partial`, checks it, and renames it.
    fn write<S: ByteSource + Sync>(
        &self,
        reader: &TextReader<S>,
        scratch: &JobScratch,
        lines: &Lines,
    ) -> Result<(), Failure> {
        let (args, plan, output) = (self.args, self.plan, self.output);
        let fail = |e| failure::build(&args.input, plan.budget, plan.machine_memory(), e);
        let memory = Memory::from(Arc::new(Budget::new(plan.budget)));
        let mut options = BuildOptions::new(self.graph, plan.threads);
        if args.ids == IdType::String {
            options.ids = options.ids.with_kind(KeyKind::Bytes);
        }
        let partial_path = PathBuf::from(format!("{}.partial", output.display()));
        let sink =
            FileSink::create(&partial_path).map_err(|e| failure::io("write", &partial_path, &e))?;
        let mut partial = Partial(Some(partial_path.clone()));
        let unfinished =
            build::build(reader, &options, &memory, scratch, sink, lines).map_err(fail)?;
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
        let (sink, report) = unfinished.commit(provenance).map_err(fail)?;
        drop(sink.into_file());

        lines.begin(Phase::Checking, None);
        let written =
            MmapSource::open(&partial_path).map_err(|e| failure::io("read", &partial_path, &e))?;
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
            .map_err(|e| failure::io("write", output, &e))?;
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

/// The first 16 hexadecimal digits of the job's fingerprint (XXH3-128): the input, the
/// output and every option that changes the graph, not the budget or the threads (R-4).
fn job_id(args: &Args, output: &Path, reader: &TextReader<FileSource>) -> String {
    let absolute = |path: &Path| {
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::canonicalize(dir).map_or_else(
            |_| path.to_path_buf(),
            |d| d.join(path.file_name().unwrap_or_default()),
        )
    };
    let fingerprint = format!(
        "{}\0{}\0{}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}",
        absolute(&args.input).display(),
        absolute(output).display(),
        reader.source().len(),
        args.source,
        args.target,
        args.directed,
        args.self_loops,
        args.duplicates,
        args.ids,
    );
    let hash = xxhash_rust::xxh3::xxh3_128(fingerprint.as_bytes());
    format!("{:016x}", hash >> 64)
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
