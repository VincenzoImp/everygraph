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

/// What was asked of `eg.read`.
struct Ask {
    path: PathBuf,
    out: Option<PathBuf>,
    text: TextOptions,
    directed: Option<bool>,
    self_loops: SelfLoops,
    duplicates: Duplicates,
    ids: IdType,
    memory: u64,
    threads: NonZeroUsize,
    temp_dir: Option<PathBuf>,
    overwrite: bool,
}

/// What `eg.read` made.
struct Made {
    graph: Shared,
    /// `None` when the cache was opened.
    report: Option<Report>,
    directed: DirectedFrom,
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
    source: Option<String>,
    target: Option<String>,
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
    let ids = if ids == "string" {
        IdType::String
    } else {
        IdType::Auto
    };
    let ask = Ask {
        text: TextOptions {
            source: source.as_deref().map(Column::from),
            target: target.as_deref().map(Column::from),
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
        memory,
        threads,
        temp_dir,
        overwrite,
    };
    let made = run(py, threads, &on_event, |events| make(&ask, events))?;
    let made = made.map_err(|e| e.into_err(py))?;
    let info = PyDict::new(py);
    let directed_from = match made.directed {
        DirectedFrom::Caller => "caller",
        DirectedFrom::ColumnNames => "column names",
        DirectedFrom::Default => "default",
    };
    info.set_item("directed_from", directed_from)?;
    info.set_item("cached", made.report.is_none())?;
    if let Some(r) = made.report {
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
    Ok((Graph(Arc::new(made.graph)), info))
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

/// What `output` holds, if it exists: `Some(true)` for the graph that `ask` would build
/// from the same, unchanged input (the cache of `docs/api-sketch.md` § 2.7),
/// `Some(false)` for another graph, and `Err` for a file that is not a graph.
fn cached(
    ask: &Ask,
    output: &Path,
    graph: &GraphOptions,
    columns: &[String; 2],
) -> Option<Result<bool, ()>> {
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
        && get("source") == Some(columns[0].as_str())
        && get("target") == Some(columns[1].as_str())))
}

fn make(ask: &Ask, events: &Events) -> Result<Made, Fail> {
    let input = &ask.path;
    let source = FileSource::open(input).map_err(|e| Fail::Os(e, input.display().to_string()))?;
    let bytes = source.len();
    let reader = TextReader::open(source, &ask.text).map_err(|e| error::text(input, e))?;
    let (directed, directed_from) = build::infer_direction(reader.layout(), ask.directed);
    let graph_options = GraphOptions {
        directed,
        self_loops: ask.self_loops,
        duplicates: ask.duplicates,
    };
    let layout = reader.layout();
    let columns = [
        layout.column_name(layout.source),
        layout.column_name(layout.target),
    ];
    let output = ask
        .out
        .clone()
        .or_else(|| (bytes >= FILE_FROM_BYTES).then(|| native::default_output(input)));
    let budget = graph::budget(ask.memory);
    if let Some(out) = output.as_deref().filter(|_| !ask.overwrite) {
        match cached(ask, out, &graph_options, &columns) {
            Some(Ok(true)) => {
                let graph = graph::open_path(out, budget)?;

                return Ok(Made {
                    graph,
                    report: None,
                    directed: directed_from,
                });
            }
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
    let root = match (&ask.temp_dir, &output) {
        (Some(dir), _) => dir.clone(),
        (None, Some(out)) => out
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
        (None, None) => std::env::temp_dir(),
    };
    let job = native::job_id(&[
        &absolute(input),
        &output
            .as_deref()
            .map_or_else(|| "memory".to_owned(), absolute),
        &columns[0],
        &columns[1],
        &format!("{graph_options:?}"),
        &format!("{:?}", ask.ids),
    ]);
    let scratch =
        JobScratch::create(&root, &job).map_err(|e| Fail::Os(e, root.display().to_string()))?;
    let memory = Memory::from(Arc::clone(&budget));
    let mut options = BuildOptions::new(graph_options, ask.threads);
    if ask.ids == IdType::String {
        options.ids = options.ids.with_kind(KeyKind::Bytes);
    }
    let target = output.as_deref().map_or(Output::Memory, Output::File);
    let job = Job {
        ask,
        options: &options,
        memory: &memory,
        scratch: &scratch,
        target,
        events,
    };
    let (written, report) = match Sample::of(&reader).map_err(|e| error::text(input, e))? {
        Some(sample) => job.build(&reader, &sample)?,
        None => {
            events.send(Event::Begin("decompressing", None));
            let plain = native::decompress(&reader, &scratch, &ask.text)
                .map_err(|e| error::write(input, ask.memory, e))?;
            events.send(Event::End("decompressing", plain.source().len()));
            let sample = Sample::of(&plain)
                .map_err(|e| error::text(input, e))?
                .unwrap_or(Sample::new(0, 0, 0, 0));
            job.build(&plain, &sample)?
        }
    };
    drop(scratch);
    if events.stopped() {
        if let Some(out) = &output {
            let _ = std::fs::remove_file(out);
        }
        return Err(Failure::new("EverygraphError")
            .text("message", "Stopped.")
            .into());
    }
    let graph = match (written, &output) {
        (Written::File(_), Some(out)) => graph::open_path(out, budget)?,
        (Written::Memory(bytes), _) => graph::in_memory(bytes, budget)?,
        (Written::File(_), None) => unreachable!("a file is written only to an output"),
    };
    Ok(Made {
        graph,
        report: Some(report),
        directed: directed_from,
    })
}

/// A build once its options are known.
struct Job<'a> {
    ask: &'a Ask,
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
        let ask = self.ask;
        let directed = self.options.graph.directed;
        let estimate = build::estimate(sample, directed, ask.threads, ask.memory);
        if ask.memory < estimate.smallest_budget {
            let refused = InsufficientBudget::new(
                "the blocks read at once",
                estimate.smallest_budget,
                ask.memory,
            );
            let threads = u64::try_from(ask.threads.get()).unwrap_or(u64::MAX);
            return Err(error::budget(&refused, ask.memory)
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
        .map_err(|e| error::write(&ask.path, ask.memory, e))?;
        Ok((written, report))
    }
}
