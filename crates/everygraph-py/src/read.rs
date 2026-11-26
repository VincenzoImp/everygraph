// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `eg.read` of a text edge list (`docs/api-sketch.md` § 3.1): a `.eg` file when `out`
//! is given or the input is large, else a graph in memory. An existing file built from
//! the same input with the same options is the cache and is opened (§ 2.7).

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use everygraph_core::{ByteSource, InsufficientBudget, Memory};
use everygraph_format::container::{Container, Input, Provenance};
use everygraph_ingest::build::{
    self, BuildOptions, DirectedFrom, Duplicates, GraphOptions, PROVENANCE, Report, Sample,
    SelfLoops,
};
use everygraph_ingest::ids::KeyKind;
use everygraph_ingest::native::{self, JobScratch, Output, Written};
use everygraph_ingest::text::{Column, IdType, TextOptions, TextReader};
use everygraph_io::FileSource;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::error::{self, Fail, Failure};
use crate::graph::{self, Graph, Shared};
use crate::run::{Event, Events, run};

/// An input at least this large is written to `NAME.eg` beside it when `out` is not
/// given; a smaller one stays in memory (`docs/api-sketch.md` § 2.8, a † value).
const FILE_FROM_BYTES: u64 = 100_000_000;

/// What was asked of `eg.read`; checked by the package.
struct Ask {
    path: PathBuf,
    out: Option<PathBuf>,
    text: TextOptions,
    directed: Option<bool>,
    self_loops: SelfLoops,
    duplicates: Duplicates,
    ids: IdType,
}

impl Ask {
    fn new(
        path: PathBuf,
        out: Option<PathBuf>,
        source: Option<&str>,
        target: Option<&str>,
        directed: Option<bool>,
        [self_loops, duplicates, ids]: [&str; 3],
    ) -> Self {
        let ids = if ids == "string" {
            IdType::String
        } else {
            IdType::Auto
        };
        Self {
            text: TextOptions {
                source: source.map(Column::from),
                target: target.map(Column::from),
                ids,
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
            ids,
            path,
            out,
            directed,
        }
    }
}

/// The input opened, and what follows from its first lines.
struct Opened {
    reader: TextReader<FileSource>,
    graph: GraphOptions,
    from: DirectedFrom,
    columns: [String; 2],
    output: Option<PathBuf>,
    /// The job's fingerprint (`docs/ux-contract.md` R-4, G-19): the input, the output and
    /// every option that changes the graph, never the budget or the threads.
    job: String,
}

fn open_input(ask: &Ask) -> Result<Opened, Fail> {
    let input = &ask.path;
    let source = FileSource::open(input).map_err(|e| Fail::Os(e, input.display().to_string()))?;
    let bytes = source.len();
    let reader = TextReader::open(source, &ask.text).map_err(|e| error::text(input, e))?;
    let layout = reader.layout();
    let (directed, from) = build::infer_direction(layout, ask.directed);
    let graph = GraphOptions {
        directed,
        self_loops: ask.self_loops,
        duplicates: ask.duplicates,
    };
    let columns = [
        layout.column_name(layout.source),
        layout.column_name(layout.target),
    ];
    let output = ask
        .out
        .clone()
        .or_else(|| (bytes >= FILE_FROM_BYTES).then(|| native::default_output(input)));
    let job = native::job_id(&[
        &absolute(input),
        &output
            .as_deref()
            .map_or_else(|| "memory".to_owned(), absolute),
        &columns[0],
        &columns[1],
        &format!("{graph:?}"),
        &format!("{:?}", ask.ids),
    ]);
    Ok(Opened {
        reader,
        graph,
        from,
        columns,
        output,
        job,
    })
}

/// What the first lines of the text edge list at `path` say, before any work (the
/// summary of `docs/ux-contract.md` § 3.4): size, compression, header, the columns of
/// the source and the target, the kind of identifier, the direction and why, where the
/// graph will go, and the job's fingerprint.
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
    let ask = Ask::new(
        path,
        out,
        source,
        target,
        directed,
        [self_loops, duplicates, ids],
    );
    let opened = open_input(&ask).map_err(|e| e.into_err(py))?;
    let layout = opened.reader.layout();
    let info = PyDict::new(py);
    info.set_item("bytes", opened.reader.source().len())?;
    info.set_item("compression", layout.compression.to_string())?;
    info.set_item("header", layout.header.clone())?;
    info.set_item("source", &opened.columns[0])?;
    info.set_item("target", &opened.columns[1])?;
    let kind = if layout.ids == IdType::String {
        "text"
    } else {
        "integers"
    };
    info.set_item("ids", kind)?;
    info.set_item("directed", opened.graph.directed)?;
    let from = match opened.from {
        DirectedFrom::Caller => "caller",
        DirectedFrom::ColumnNames => "column names",
        DirectedFrom::Default => "default",
    };
    info.set_item("directed_from", from)?;
    info.set_item("output", opened.output)?;
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
    let ask = Ask::new(
        path,
        out,
        source,
        target,
        directed,
        [self_loops, duplicates, ids],
    );
    let run_on = Run {
        memory,
        threads,
        temp_dir,
        overwrite,
    };
    let made = run(py, threads, &on_event, |events| make(&ask, &run_on, events))?;
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
    Ok((Graph(Arc::new(graph)), info))
}

/// How the build runs: these never change the graph.
struct Run {
    memory: u64,
    threads: NonZeroUsize,
    temp_dir: Option<PathBuf>,
    overwrite: bool,
}

fn ns(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
}

fn absolute(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

/// The provenance of a build of `ask`: its input's name, size and time, and the way
/// identifiers were read. The build adds its own parameters.
fn provenance(ask: &Ask) -> Provenance {
    let metadata = std::fs::metadata(&ask.path).ok();
    let mut provenance = Provenance {
        engine: format!("everygraph {}", env!("CARGO_PKG_VERSION")),
        started: ns(SystemTime::now()),
        inputs: vec![Input {
            name: absolute(&ask.path),
            bytes: metadata.as_ref().map_or(0, std::fs::Metadata::len),
            modified: metadata.and_then(|m| m.modified().ok()).and_then(ns),
            ..Input::default()
        }],
        ..Provenance::default()
    };
    let ids = if ask.ids == IdType::String {
        "string"
    } else {
        "auto"
    };
    provenance.parameters.insert("ids".into(), ids.into());
    provenance
}

/// What `output` holds, if it exists: `Ok(true)` for the graph that `ask` would build
/// from the same, unchanged input (the cache of `docs/api-sketch.md` § 2.7), `Ok(false)`
/// for another graph, and `Err` for a file that is not a graph.
fn cached(ask: &Ask, opened: &Opened, output: &Path) -> Option<Result<bool, ()>> {
    let source = FileSource::open(output).ok()?;
    let Ok(file) = Container::open(source) else {
        return Some(Err(()));
    };
    let Ok(stored) = file.read_provenance(PROVENANCE) else {
        return Some(Ok(false));
    };
    let wanted = provenance(ask);
    let identity = |p: &Provenance| {
        p.inputs
            .first()
            .map(|i| (i.name.clone(), i.bytes, i.modified))
    };
    let get = |key: &str| stored.parameters.get(key).map(String::as_str);
    let graph = opened.graph;
    let self_loops = if graph.self_loops == SelfLoops::Drop {
        "drop"
    } else {
        "keep"
    };
    let duplicates = match graph.duplicates {
        Duplicates::Events => "events",
        Duplicates::Collapse => "collapse",
        Duplicates::Sum => "sum",
    };
    Some(Ok(identity(&stored) == identity(&wanted)
        && get("directed")
            == Some(if graph.directed { "true" } else { "false" })
        && get("self_loops") == Some(self_loops)
        && get("duplicates") == Some(duplicates)
        && get("ids")
            == wanted.parameters.get("ids").map(String::as_str)
        && get("source") == Some(opened.columns[0].as_str())
        && get("target") == Some(opened.columns[1].as_str())))
}

/// The graph of `ask`, and the build's report (`None` for the cache).
fn make(ask: &Ask, run: &Run, events: &Events) -> Result<(Shared, Option<Report>), Fail> {
    let input = &ask.path;
    let opened = open_input(ask)?;
    let output = opened.output.as_deref();
    let budget = graph::budget(run.memory);
    if let Some(out) = output.filter(|_| !run.overwrite) {
        match cached(ask, &opened, out) {
            Some(Ok(true)) => return Ok((graph::open_path(out, budget)?, None)),
            Some(holds) => {
                let holds = if holds.is_ok() {
                    "other graph"
                } else {
                    "not ours"
                };
                return Err(Failure::new("OutputExists")
                    .text("path", out.display())
                    .text("holds", holds)
                    .into());
            }
            None => {}
        }
    }
    let root = match (&run.temp_dir, output) {
        (Some(dir), _) => dir.clone(),
        (None, Some(out)) => out
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
        (None, None) => std::env::temp_dir(),
    };
    let scratch = JobScratch::create(&root, &opened.job)
        .map_err(|e| Fail::Os(e, root.display().to_string()))?;
    let memory = Memory::from(Arc::clone(&budget));
    let mut options = BuildOptions::new(opened.graph, run.threads);
    if ask.ids == IdType::String {
        options.ids = options.ids.with_kind(KeyKind::Bytes);
    }
    // The graph takes its name only once the call is known not to be stopped, so that a
    // stopped call leaves a file in the way as it was.
    let staged = output.map(|o| PathBuf::from(format!("{}.staged", o.display())));
    let job = Job {
        ask,
        run,
        options: &options,
        memory: &memory,
        scratch: &scratch,
        target: staged.as_deref().map_or(Output::Memory, Output::File),
        events,
    };
    let reader = &opened.reader;
    let (written, report) = match Sample::of(reader).map_err(|e| error::text(input, e))? {
        Some(sample) => job.build(reader, &sample)?,
        None => {
            // A compressed input is first copied as plain text: part of `reading`.
            events.send(Event::Begin("reading", None));
            let plain = native::decompress(reader, &scratch, &ask.text)
                .map_err(|e| error::write(input, run.memory, e))?;
            let sample = Sample::of(&plain)
                .map_err(|e| error::text(input, e))?
                .unwrap_or(Sample::new(0, 0, 0, 0));
            job.build(&plain, &sample)?
        }
    };
    drop(scratch);
    if let (Some(staged), Some(out)) = (&staged, output) {
        if events.stopped() {
            let _ = std::fs::remove_file(staged);
        } else {
            everygraph_io::persist(staged, out)
                .map_err(|e| Fail::Os(e, out.display().to_string()))?;
        }
    }
    if events.stopped() {
        return Err(Failure::new("EverygraphError")
            .text("message", "Stopped.")
            .into());
    }
    let graph = match written {
        Written::File(_) => graph::open_path(output.unwrap_or(input), budget)?,
        Written::Memory(bytes) => graph::in_memory(bytes, budget)?,
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
    target: Output<'a>,
    events: &'a Events,
}

impl Job<'_> {
    /// Refuses, before any work, a budget below the smallest the build can work in;
    /// else builds the graph of the plain text `reader`, sampled as `sample`.
    fn build<S: ByteSource + Sync>(
        &self,
        reader: &TextReader<S>,
        sample: &Sample,
    ) -> Result<(Written, Report), Fail> {
        let (ask, run) = (self.ask, self.run);
        let directed = self.options.graph.directed;
        let estimate = build::estimate(sample, directed, run.threads, run.memory);
        if run.memory < estimate.smallest_budget {
            let refused = InsufficientBudget::new(
                "the blocks read at once",
                estimate.smallest_budget,
                run.memory,
            );
            let threads = u64::try_from(run.threads.get()).unwrap_or(u64::MAX);
            return Err(error::budget(&refused, run.memory)
                .count("threads", threads)
                .into());
        }
        let (written, report, _) = native::write(
            reader,
            self.options,
            self.memory,
            self.scratch,
            self.target,
            provenance(ask),
            self.events,
        )
        .map_err(|e| error::write(&ask.path, run.memory, e))?;
        Ok((written, report))
    }
}
