// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `eg.read` of a text edge list (`docs/api-sketch.md` § 3.1): a `.eg` file when `out`
//! is given or the plain text is large, else a graph in memory. An existing file built
//! from the same input with the same options is the cache and is opened; any other file
//! in the way is never replaced unless `overwrite` says so (§ 2.7).

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use everygraph_core::{Budget, ByteSource, InsufficientBudget, Memory, MemoryKind};
use everygraph_format::container::{Input, Provenance};
use everygraph_ingest::build::{
    self, BuildOptions, DirectedFrom, Duplicates, GraphOptions, Report, Sample, SelfLoops,
};
use everygraph_ingest::ids::KeyKind;
use everygraph_ingest::native::{self, Held, JobScratch, Output, WriteError, Written};
use everygraph_ingest::text::{Column, Compression, IdType, TextOptions, TextReader};
use everygraph_io::FileSource;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::error::{self, Fail, Failure};
use crate::graph::{self, Graph, Shared};
use crate::run::{Event, Events, run};

/// Plain text at least this large is built into `NAME.eg` beside the input when `out`
/// is not given; less stays in memory (`docs/api-sketch.md` § 2.8, a † value).
const FILE_FROM_BYTES: u64 = 100_000_000;

/// What was asked of `eg.read`; checked by the package.
struct Ask {
    path: PathBuf,
    out: Option<PathBuf>,
    text: TextOptions,
    directed: Option<bool>,
    self_loops: SelfLoops,
    duplicates: Duplicates,
}

impl Ask {
    fn new(
        path: PathBuf,
        out: Option<PathBuf>,
        [source, target]: [Option<&str>; 2],
        directed: Option<bool>,
        [self_loops, duplicates, ids]: [&str; 3],
    ) -> Self {
        Self {
            text: TextOptions {
                source: source.map(Column::from),
                target: target.map(Column::from),
                ids: if ids == "string" {
                    IdType::String
                } else {
                    IdType::Auto
                },
                ..TextOptions::default()
            },
            self_loops: if self_loops == "keep" {
                SelfLoops::Keep
            } else {
                SelfLoops::Drop
            },
            duplicates: match duplicates {
                "collapse" => Duplicates::Collapse,
                "sum" => Duplicates::Sum,
                _ => Duplicates::Events,
            },
            path,
            out,
            directed,
        }
    }
}

/// Where the graph goes, as far as it is known before the input is decompressed.
#[derive(Clone, Debug)]
enum Target {
    File(PathBuf),
    Memory,
    /// A compressed input without `out`: `NAME.eg` if its plain text is large.
    Undecided(PathBuf),
}

/// The input opened, and what follows from its first lines.
struct Opened {
    reader: TextReader<FileSource>,
    graph: GraphOptions,
    from: DirectedFrom,
    /// The parameters its build records (`build::parameters`): the cache's key, with
    /// the input's identity.
    parameters: BTreeMap<String, String>,
    /// The input's identity (R-6), taken once, now.
    identity: Input,
    target: Target,
    /// The job's fingerprint (`docs/ux-contract.md` R-4, G-19): the input, the output and
    /// every option that changes the graph, never the budget or the threads.
    job: String,
}

fn open_input(ask: &Ask) -> Result<Opened, Fail> {
    let input = &ask.path;
    let os = |e| Fail::Os(e, input.display().to_string());
    let identity = native::identity(input).map_err(os)?;
    let source = FileSource::open(input).map_err(os)?;
    let reader = TextReader::open(source, &ask.text).map_err(|e| error::text(input, e))?;
    let layout = reader.layout();
    let (directed, from) = build::infer_direction(layout, ask.directed);
    let graph = GraphOptions {
        directed,
        self_loops: ask.self_loops,
        duplicates: ask.duplicates,
    };
    let parameters = build::parameters(&graph, layout);
    let target = match (&ask.out, layout.compression) {
        (Some(out), _) => Target::File(native::canonical(out)),
        (None, Compression::None) if identity.bytes < FILE_FROM_BYTES => Target::Memory,
        (None, Compression::None) => Target::File(native::default_output(input)),
        (None, _) => Target::Undecided(native::default_output(input)),
    };
    let output = match &ask.out {
        Some(out) => native::canonical(out).display().to_string(),
        None => "beside the input, or memory".into(),
    };
    let mut parts = vec![identity.name.clone(), output];
    parts.extend(parameters.iter().map(|(k, v)| format!("{k}={v}")));
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let job = native::job_id(&parts);
    Ok(Opened {
        reader,
        graph,
        from,
        parameters,
        identity,
        target,
        job,
    })
}

/// What the first lines of the text edge list at `path` say, before any work (the
/// summary of `docs/ux-contract.md` § 3.4): size, compression, header, the columns of
/// the source and the target, the kind of identifier, the direction and why, and the
/// job's fingerprint.
#[pyfunction]
#[pyo3(signature = (path, *, out, source, target, directed, self_loops, duplicates, ids))]
#[expect(clippy::too_many_arguments, reason = "the keywords of eg.read")]
pub(crate) fn inspect<'py>(
    py: Python<'py>,
    path: PathBuf,
    out: Option<PathBuf>,
    source: Option<&str>,
    target: Option<&str>,
    directed: Option<bool>,
    self_loops: &str,
    duplicates: &str,
    ids: &str,
) -> PyResult<Bound<'py, PyDict>> {
    let columns = [source, target];
    let ask = Ask::new(path, out, columns, directed, [self_loops, duplicates, ids]);
    let opened = open_input(&ask).map_err(|e| e.into_err(py))?;
    let layout = opened.reader.layout();
    let info = PyDict::new(py);
    info.set_item("bytes", opened.identity.bytes)?;
    info.set_item("compression", layout.compression.to_string())?;
    info.set_item("header", layout.header.clone())?;
    for key in ["source", "target", "ids"] {
        info.set_item(key, &opened.parameters[key])?;
    }
    info.set_item("directed", opened.graph.directed)?;
    let from = match opened.from {
        DirectedFrom::Caller => "caller",
        DirectedFrom::ColumnNames => "column names",
        DirectedFrom::Default => "default",
    };
    info.set_item("directed_from", from)?;
    info.set_item("job", opened.job)?;
    Ok(info)
}

/// Reads the text edge list at `path`; the arguments are those of the package's
/// `everygraph.read`, already checked by it. Returns the graph and what was read.
#[pyfunction]
#[pyo3(signature = (path, *, out, source, target, directed, self_loops, duplicates, ids,
                    memory, threads, temp_dir, overwrite, on_event))]
#[expect(clippy::too_many_arguments, reason = "the keywords of eg.read")]
pub(crate) fn read<'py>(
    py: Python<'py>,
    path: PathBuf,
    out: Option<PathBuf>,
    source: Option<&str>,
    target: Option<&str>,
    directed: Option<bool>,
    self_loops: &str,
    duplicates: &str,
    ids: &str,
    memory: u64,
    threads: NonZeroUsize,
    temp_dir: Option<PathBuf>,
    overwrite: bool,
    on_event: Bound<'py, PyAny>,
) -> PyResult<(Graph, Bound<'py, PyDict>)> {
    let columns = [source, target];
    let ask = Ask::new(path, out, columns, directed, [self_loops, duplicates, ids]);
    let budget = graph::budget(memory);
    let run_on = Run {
        budget: Arc::clone(&budget),
        threads,
        temp_dir,
        overwrite,
    };
    let made = run(py, threads, &budget, &on_event, |events| {
        make(&ask, &run_on, events)
    })?;
    let (graph, report) = made.map_err(|e| e.into_err(py))?;
    let info = PyDict::new(py);
    info.set_item("cached", report.is_none())?;
    if let Some(r) = report {
        for (key, value) in [
            ("rows", r.rows),
            ("rejected", r.rejected),
            ("dropped_self_loops", r.dropped_self_loops),
            ("dropped_duplicates", r.dropped_duplicates),
            ("merged", r.merged),
            ("spilled_bytes", r.spilled_bytes),
        ] {
            info.set_item(key, value)?;
        }
    }
    info.set_item("peak_memory_bytes", budget.peak())?;
    Ok((Graph(Arc::new(graph)), info))
}

/// How the build runs: these never change the graph.
struct Run {
    budget: Arc<Budget>,
    threads: NonZeroUsize,
    temp_dir: Option<PathBuf>,
    overwrite: bool,
}

/// The error that a stopped call carries: never shown, since the call raises what
/// stopped it.
fn stopped() -> Fail {
    Failure::new("EverygraphError")
        .text("message", "Stopped.")
        .into()
}

fn output_exists(out: &Path, held: Held) -> Fail {
    let holds = if held == Held::OtherGraph {
        "other graph"
    } else {
        "not ours"
    };
    Failure::new("OutputExists")
        .text("path", out.display())
        .text("holds", holds)
        .into()
}

/// The graph of `ask`, and the build's report (`None` for the cache).
fn make(ask: &Ask, run: &Run, events: &Events) -> Result<(Shared, Option<Report>), Fail> {
    let input = &ask.path;
    let limit = run.budget.limit();
    let opened = open_input(ask)?;
    if events.stopped() {
        return Err(stopped());
    }
    // What is in the way of a file the build may write: the cache is opened, anything
    // else refused, unless `overwrite`. For an undecided target the refusal waits until
    // the plain size says a file is made.
    let mut blocked = None;
    if let Target::File(out) | Target::Undecided(out) = &opened.target {
        let held = if run.overwrite {
            Held::Nothing
        } else {
            native::held(out, &opened.identity, &opened.parameters)
        };
        match (held, &opened.target) {
            (Held::Same, _) => {
                return Ok((graph::open_path(out, Arc::clone(&run.budget))?, None));
            }
            (Held::Nothing, _) => {}
            (other, Target::File(_)) => return Err(output_exists(out, other)),
            (other, _) => blocked = Some(other),
        }
    }
    let root = match (&run.temp_dir, &opened.target) {
        (Some(dir), _) => dir.clone(),
        (None, Target::File(out) | Target::Undecided(out)) => out
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
        (None, Target::Memory) => std::env::temp_dir(),
    };
    let scratch = JobScratch::create(&root, &opened.job)
        .map_err(|e| Fail::Os(e, root.display().to_string()))?;
    let memory = Memory::from(Arc::clone(&run.budget));
    let mut options = BuildOptions::new(opened.graph, run.threads);
    if opened.parameters["ids"] == "string" {
        options.ids = options.ids.with_kind(KeyKind::Bytes);
    }
    let job = Job {
        ask,
        run,
        options: &options,
        memory: &memory,
        scratch: &scratch,
        events,
        identity: &opened.identity,
    };
    let reader = &opened.reader;
    let (written, report) = match Sample::of(reader).map_err(|e| error::text(input, e))? {
        Some(sample) => {
            let target = match &opened.target {
                Target::File(out) => Output::File(out),
                _ => Output::Memory,
            };
            job.build(reader, &sample, target)?
        }
        None => {
            // A compressed input is first copied as plain text: part of `reading`.
            events.send(Event::Begin("reading", None));
            let plain = native::decompress(reader, &scratch, &ask.text)
                .map_err(|e| error::write(input, limit, e))?;
            if events.stopped() {
                return Err(stopped());
            }
            let plain_bytes = plain.source().len();
            // Scratch on a file system held in memory is memory (P-18).
            let _copy = everygraph_io::memory_backed(&root)
                .then(|| {
                    let purpose = "the plain copy of the input, in scratch held in memory";
                    memory.reserve(purpose, MemoryKind::Anonymous, plain_bytes)
                })
                .transpose()
                .map_err(|refused| {
                    error::budget(&refused, limit).text("temp_dir", root.display())
                })?;
            let target = match &opened.target {
                Target::File(out) => Output::File(out),
                Target::Undecided(out) if plain_bytes >= FILE_FROM_BYTES => match blocked {
                    Some(held) => return Err(output_exists(out, held)),
                    None => Output::File(out),
                },
                _ => Output::Memory,
            };
            let sample = Sample::of(&plain)
                .map_err(|e| error::text(input, e))?
                .unwrap_or(Sample::new(0, 0, 0, 0));
            job.build(&plain, &sample, target)?
        }
    };
    drop(scratch);
    let graph = match written {
        Written::File(path, _) => graph::open_path(&path, Arc::clone(&run.budget))?,
        Written::Memory(bytes) => graph::in_memory(bytes, Arc::clone(&run.budget))?,
    };
    Ok((graph, Some(report)))
}

/// A build once its options are known.
struct Job<'a> {
    ask: &'a Ask,
    run: &'a Run,
    options: &'a BuildOptions,
    memory: &'a Memory,
    scratch: &'a JobScratch,
    events: &'a Events,
    identity: &'a Input,
}

impl Job<'_> {
    /// Refuses, before any work, a budget below the smallest the build can work in;
    /// else builds the graph of the plain text `reader`, sampled as `sample`, into
    /// `target`. A file takes its name only if the call was not stopped.
    fn build<S: ByteSource + Sync>(
        &self,
        reader: &TextReader<S>,
        sample: &Sample,
        target: Output<'_>,
    ) -> Result<(Written, Report), Fail> {
        let (ask, run) = (self.ask, self.run);
        let limit = run.budget.limit();
        let directed = self.options.graph.directed;
        let estimate = build::estimate(sample, directed, run.threads, limit);
        if limit < estimate.smallest_budget {
            let refused =
                InsufficientBudget::new("the blocks read at once", estimate.smallest_budget, limit);
            let threads = u64::try_from(run.threads.get()).unwrap_or(u64::MAX);
            return Err(error::budget(&refused, limit)
                .count("threads", threads)
                .into());
        }
        let provenance = Provenance {
            engine: format!("everygraph {}", env!("CARGO_PKG_VERSION")),
            started: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_nanos()).ok()),
            inputs: vec![self.identity.clone()],
            ..Provenance::default()
        };
        let keep = || !self.events.stopped();
        let built = native::write(
            reader,
            self.options,
            self.memory,
            self.scratch,
            target,
            provenance,
            self.events,
            &keep,
        );
        match built {
            Ok((written, report, _)) => Ok((written, report)),
            Err(WriteError::Withdrawn) => Err(stopped()),
            Err(e) => Err(error::write(&ask.path, limit, e)),
        }
    }
}
