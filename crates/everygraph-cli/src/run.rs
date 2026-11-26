// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph run NAME GRAPH` (`docs/api-sketch.md` § 5.1–5.3): a whole-graph
//! algorithm. A result on the whole graph is looked up in the graph file before it is
//! computed, and kept there after (§ 2.8); `bfs` is never kept. The table goes to
//! standard output — aligned on a terminal, tab-separated when piped — or, with
//! `--out`, to a file; with `--json`, the result document.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use everygraph_core::{ByteSource, Memory, MemoryError, SegVec, par};
use everygraph_format::container::{
    Container, ContainerError, GraphIdentity, Provenance, ResultKey, StoredResult, Value as Stored,
    ValueType, Writer,
};
use everygraph_ingest::graph::Graph;
use everygraph_ingest::ids::Key;
use everygraph_io::{FileSink, MmapSource};
use everygraph_kernels::edge_map::{Compressed, Error, Plan, Settings};
use everygraph_kernels::pagerank::PageRank;
use everygraph_kernels::run::{self, Column, NAMES, Operation, Output, Precision, RunError};
use serde_json::{Map, Value, json};

use crate::args::{self as cli, Arg, Common};
use crate::failure::{self, Failure};
use crate::graph;
use crate::ingest::is_graph;
use crate::machine::{self, Machine};
use crate::progress::{Lines, now_ns};
use crate::render::{Style, count, counted, duration, size};
use crate::table::{self, Nodes, SHOWN, Shown};

const HELP: &str = "\
Run a whole-graph algorithm.

Usage
  everygraph run NAME GRAPH [NODE] [OPTIONS]

Names
  pagerank              the PageRank of each node
  components            the weakly connected component of each node, numbered
                        from 0 by decreasing size
  kcore                 the core number of each node
  degrees               the number of distinct neighbours of each node
  bfs                   hops from NODE to every node

Options
  --top N               the N largest values, ranked (pagerank, kcore, degrees);
                        the first N rows otherwise (default: 10 on a terminal)
  --out FILE            write the whole result to FILE: .parquet, .arrow, .csv
                        or .json
  --overwrite           replace FILE if it is in the way
  --memory SIZE         the memory budget, for example 8GB
  --threads N           the most threads to use
  --json                print the result document, with nothing around it
  --progress plain      progress as one line per finished phase, never redrawn
  -h, --help            this text

Options of pagerank
  --damping X           the probability of following an edge (default: 0.85)
  --tol X               stop when the sum of changes is below X (default: 1e-6)
  --max-iter N          iterations at most (default: 100)
  --precision full|low  64-bit or 32-bit state (default: full)

Options of bfs
  --max-hops N          stop at this distance
  --direction out|in|both
                        the edges followed on a directed graph (default: out)
";

/// What was asked.
struct Request {
    op: Operation,
    graph: PathBuf,
    node: Option<String>,
    top: Option<usize>,
    common: Common,
}

/// Runs the command; returns the exit status.
pub(crate) fn main(raw: &[OsString]) -> u8 {
    let json = raw.iter().any(|a| a == "--json");
    match parse(raw).and_then(|r| r.map_or(Ok(0), |r| execute(&r))) {
        Ok(status) => status,
        Err(failure) => failure.report(json),
    }
}

fn unsupported(what: String, why: &str) -> Failure {
    Failure::new("Unsupported", what, failure::USAGE)
        .why(vec![why.to_owned()])
        .ways(vec![(
            "everygraph run --help".into(),
            "what this version can run".into(),
        )])
        .on_existing_file()
}

fn parse(raw: &[OsString]) -> Result<Option<Request>, Failure> {
    let mut it = cli::Args::new(raw, "run");
    let mut plain = Vec::new();
    let mut common = Common::default();
    let mut top = None;
    let mut pagerank = PageRank::default();
    let mut precision = Precision::Full;
    let mut max_hops = None;
    let mut outward = true;
    let mut given: Vec<&str> = Vec::new();
    while let Some(arg) = it.next()? {
        let option = match arg {
            Arg::Plain(p) => {
                plain.push(p);
                continue;
            }
            Arg::Option(help) if cli::is_help(help) => {
                print!("{HELP}");
                return Ok(None);
            }
            Arg::Option(option) => option,
        };
        if common.take(option, &mut it)? {
            continue;
        }
        match option {
            "--top" => top = Some(it.parsed(option, "a count of rows")?),
            "--damping" => pagerank.damping = it.parsed(option, "a number")?,
            "--tol" => pagerank.tol = it.parsed(option, "a number")?,
            "--max-iter" => pagerank.max_iter = it.parsed(option, "a count")?,
            "--precision" => {
                precision = it.choice(
                    option,
                    &[("full", Precision::Full), ("low", Precision::Low)],
                )?;
            }
            "--max-hops" => max_hops = Some(it.parsed(option, "a count")?),
            "--direction" => {
                outward = it.choice(option, &[("out", true), ("in", false), ("both", false)])?;
            }
            "--strong" | "--personalize" | "--dry-run" | "--weight" | "--temp-dir" => {
                return Err(unsupported(
                    format!("{option} cannot be used with run yet."),
                    "This version computes unweighted results on the whole graph, \
                     with no planner and no strong components.",
                ));
            }
            other => return Err(it.unknown(other)),
        }
        given.push(option);
    }
    let mut plain = plain.into_iter();
    let name = plain
        .next()
        .ok_or_else(|| it.usage("run needs a NAME and a GRAPH.".into()))?
        .to_string_lossy()
        .into_owned();
    let graph = PathBuf::from(
        plain
            .next()
            .ok_or_else(|| it.usage(format!("run {name} needs a GRAPH.")))?,
    );
    let node = plain.next().map(|n| n.to_string_lossy().into_owned());
    if let Some(extra) = plain.next() {
        return Err(it.usage(format!(
            "run takes a NAME, a GRAPH and, for bfs, a NODE; {} is one more.",
            extra.to_string_lossy()
        )));
    }
    let op = match name.as_str() {
        "pagerank" => Operation::PageRank {
            params: pagerank,
            precision,
        },
        "components" => Operation::Components,
        "kcore" => Operation::KCore,
        "degrees" => Operation::Degrees,
        "bfs" => Operation::Bfs {
            source: 0,
            max_hops,
        },
        "triangles" | "communities" => {
            return Err(unsupported(
                format!("{name} cannot be run yet."),
                &format!("This version runs {}.", NAMES.join(", ")),
            ));
        }
        other => {
            return Err(it.usage(format!(
                "Unknown algorithm {other}: run takes {}.",
                NAMES.join(", ")
            )));
        }
    };
    let own: &[&str] = match op {
        Operation::PageRank { .. } => &["--damping", "--tol", "--max-iter", "--precision"],
        Operation::Bfs { .. } => &["--max-hops", "--direction"],
        _ => &[],
    };
    if let Some(stray) = given.iter().find(|o| **o != "--top" && !own.contains(o)) {
        return Err(it.usage(format!("{stray} is not an option of run {name}.")));
    }
    match (&op, &node) {
        (Operation::Bfs { .. }, None) => return Err(it.usage("run bfs needs a NODE.".into())),
        (Operation::Bfs { .. }, Some(_)) => {}
        (_, Some(extra)) => {
            return Err(it.usage(format!("run {name} takes no NODE; {extra} is one more.")));
        }
        _ => {}
    }
    if !outward {
        return Err(unsupported(
            "--direction in and both cannot be used yet.".into(),
            "They need the in-adjacency, which no graph file holds yet.",
        ));
    }
    Ok(Some(Request {
        op,
        graph,
        node,
        top,
        common,
    }))
}

fn execute(req: &Request) -> Result<u8, Failure> {
    let started = Instant::now();
    let started_at = now_ns();
    let machine = Machine::probe();
    let budget = machine.budget(&req.common)?;
    let threads = machine.threads(&req.common);
    if let Some(out) = &req.common.out {
        table::format_of(out)?;
        if out.exists() && !req.common.overwrite {
            return Err(failure::output_exists(out, is_graph(out)));
        }
    }
    let memory = machine::memory(budget);
    let graph = graph::open(&req.graph, &memory, budget)?;
    let nodes = Nodes::new(
        graph
            .dictionary(memory.accountant())
            .map_err(|e| failure::corrupt(&req.graph, &e))?,
    );
    let mut op = req.op;
    if let (Operation::Bfs { source, .. }, Some(node)) = (&mut op, &req.node) {
        *source = lookup(&nodes, node, &req.graph)?;
    }
    let directed = graph.directed();
    let identity = graph.file().graph_identity();
    let key = result_key(identity, &op)
        .map_err(|e| failure::internal(&format!("The result key cannot be made: {e}.")))?;
    let lines = Lines::new(Style::stderr().progress(req.common.plain));
    let name = op.name();
    lines.start(name);
    let cached = if op.stored() {
        cached(graph.file(), &key, op.column(directed), &memory, &req.graph)?
    } else {
        None
    };
    let is_cached = cached.is_some();
    let output = match cached {
        Some(output) => {
            lines.seconds(name);
            lines.kept(name, &detail(&op, &output));
            output
        }
        None => {
            let output = par::with_threads(threads, || compute(&graph, &op, &memory))
                .map_err(|e| failure::internal(&format!("Cannot start the threads: {e}.")))?
                .map_err(|e| kernel_failure(e, &op, &req.graph, budget, machine.total()))?;
            lines.done_if_long(name, &detail(&op, &output));
            output
        }
    };
    let mut stored_in = None;
    if op.stored() {
        if is_cached {
            stored_in = Some(req.graph.display().to_string());
        } else {
            let about = Provenance {
                engine: format!("everygraph {}", env!("CARGO_PKG_VERSION")),
                started: started_at,
                finished: now_ns(),
                ..Provenance::default()
            };
            match store(&req.graph, &key, &about, &output) {
                Ok(()) => stored_in = Some(req.graph.display().to_string()),
                Err(why) => failure::warning(
                    &format!("The result could not be kept in {}.", req.graph.display()),
                    &why,
                    &[],
                    "It is shown here, and computed again next time.",
                ),
            }
        }
    }
    let shown = shown(&op, &output, graph.nodes(), req);
    if let Some(out) = &req.common.out {
        let bytes = table::write_result(out, &req.graph, &nodes, &output.columns)?;
        stored_in = Some(out.display().to_string());
        if !req.common.json {
            let style = Style::stderr();
            let dot = style.dot();
            let name = out
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let line = format!(
                "{} {}{dot}{}{dot}{}{dot}{}\n",
                style.ok(),
                style.bold(&name),
                size(bytes),
                counted(u64::from(graph.nodes()), "row", "rows"),
                duration(started.elapsed().as_secs_f64())
            );
            let _ = io::stderr().write_all(line.as_bytes());
        }
    }
    let corrupt = |e| failure::corrupt(&req.graph, &e);
    let stdout = Style::stdout();
    if req.common.json {
        let doc = document(
            req, &op, &output, &shown, &nodes, stored_in, is_cached, directed,
        )
        .map_err(corrupt)?;
        println!("{doc}");
    } else if req.common.out.is_none() {
        let text = if stdout.piped {
            shown.piped(&nodes).map_err(corrupt)?
        } else {
            let mut text = summary_text(&op, &output, stdout);
            text.push_str(&shown.aligned(&nodes, stdout).map_err(corrupt)?);
            text
        };
        let _ = io::stdout().write_all(text.as_bytes());
        if !stdout.piped {
            footer(
                req,
                &op,
                &shown,
                graph.nodes(),
                is_cached,
                stored_in.is_some(),
            );
        }
    }
    Ok(0)
}

/// The key `op`'s result on the graph `identity` is stored under: every run is
/// deterministic.
fn result_key(identity: GraphIdentity, op: &Operation) -> Result<ResultKey, ContainerError> {
    op.parameters().into_iter().try_fold(
        ResultKey::new(identity, op.name(), op.version(), true)?,
        |key, (name, value)| key.with(name, &value),
    )
}

/// The node `text` names, as the dictionary resolves it.
fn lookup<S: ByteSource>(nodes: &Nodes<'_, S>, text: &str, graph: &Path) -> Result<u32, Failure> {
    let found = nodes
        .dictionary()
        .lookup(Key::from(text))
        .map_err(|e| failure::corrupt(graph, &e))?;
    found.map(everygraph_core::NodeId::get).ok_or_else(|| {
        Failure::new(
            "UnknownNode",
            format!("{text} is not in {}.", graph.display()),
            failure::UNKNOWN_NODE,
        )
        .key("node", text)
        .ways(vec![(
            "check the identifier: it is matched as the input wrote it".into(),
            String::new(),
        )])
        .on_existing_file()
    })
}

/// Runs `op` on the graph's out-lists.
fn compute(graph: &Graph<MmapSource>, op: &Operation, memory: &Memory) -> Result<Output, RunError> {
    let data = graph
        .data()
        .map_err(|_| RunError::Kernel(Error::Invalid("the graph has no adjacency data")))?;
    let source = Compressed::mapped(graph.index(), &data)?;
    Plan::with(&source, memory, Settings::default(), |plan| {
        run::run(op, &plan, graph.directed(), memory)
    })
}

/// The result stored under `key`, if there is one for this graph.
fn cached(
    file: &Container<MmapSource>,
    key: &ResultKey,
    name: &'static str,
    memory: &Memory,
    path: &Path,
) -> Result<Option<Output>, Failure> {
    let corrupt = |e: &dyn std::fmt::Display| failure::corrupt(path, e);
    let Some(stored) = file.find_result(key).map_err(|e| corrupt(&e))? else {
        return Ok(None);
    };
    file.verify_result(&stored).map_err(|e| corrupt(&e))?;
    let len = usize::try_from(stored.len()).map_err(|e| corrupt(&e))?;
    let held = |e: MemoryError| match e {
        MemoryError::Budget(e) => {
            failure::memory("reading of the result", memory.available(), None, &e)
                .on_existing_file()
        }
        other => corrupt(&other),
    };
    let column = match stored.value_type() {
        ValueType::U32 => {
            let mut values = SegVec::try_with_len(memory, "result", len, 0_u32).map_err(held)?;
            read(&stored, &mut values).map_err(|e| corrupt(&e))?;
            Column::U32 { values, null: None }
        }
        ValueType::F64 => {
            let mut values = SegVec::try_with_len(memory, "result", len, 0.0_f64).map_err(held)?;
            read(&stored, &mut values).map_err(|e| corrupt(&e))?;
            Column::F64(values)
        }
        ValueType::F32 => {
            let mut values = SegVec::try_with_len(memory, "result", len, 0.0_f32).map_err(held)?;
            read(&stored, &mut values).map_err(|e| corrupt(&e))?;
            Column::F32(values)
        }
        _ => return Ok(None),
    };
    Ok(Some(Output {
        columns: vec![(name, column)],
        iterations: None,
        converged: None,
    }))
}

fn read<T: Stored>(
    stored: &StoredResult<'_, MmapSource>,
    values: &mut SegVec<T>,
) -> Result<(), ContainerError> {
    let mut first = 0;
    for segment in values.segments_mut() {
        stored.read(first, segment)?;
        first += segment.len() as u64;
    }
    Ok(())
}

/// Commits `output` to the graph file as the result of `key`; the reason it could not.
fn store(path: &Path, key: &ResultKey, about: &Provenance, output: &Output) -> Result<(), String> {
    let sink = FileSink::open(path).map_err(|e| e.to_string())?;
    let mut writer = Writer::update(sink).map_err(|e| e.to_string())?;
    let Some((_, column)) = output.columns.first() else {
        return Ok(());
    };
    let one = "a result longer than one segment";
    match column {
        Column::U32 { values, .. } | Column::Nodes { values, .. } => {
            writer.add_result(key, about, values.as_slice().ok_or(one)?)
        }
        Column::F64(values) => writer.add_result(key, about, values.as_slice().ok_or(one)?),
        Column::F32(values) => writer.add_result(key, about, values.as_slice().ok_or(one)?),
    }
    .map_err(|e| e.to_string())?;
    writer.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// What a kernel's failure means for the person who ran it.
fn kernel_failure(
    e: RunError,
    op: &Operation,
    graph: &Path,
    budget: u64,
    machine: Option<u64>,
) -> Failure {
    match e {
        RunError::Unsupported(why) => unsupported(
            format!("{} cannot be done on {}.", op.name(), graph.display()),
            &format!("{}: {why}.", op.name()),
        ),
        RunError::Kernel(Error::Memory(everygraph_core::MemoryError::Budget(refused))) => {
            failure::memory(op.name(), budget, machine, &refused).on_existing_file()
        }
        RunError::Kernel(Error::Argument(why)) => Failure::new(
            "UsageError",
            format!("{} cannot use its parameters: {why}.", op.name()),
            failure::USAGE,
        )
        .ways(vec![("everygraph run --help".into(), String::new())])
        .on_existing_file(),
        RunError::Kernel(other) => failure::corrupt(graph, &other),
    }
}

/// The measure `--top` ranks by, if the operation has one.
fn ranked(op: &Operation) -> bool {
    matches!(
        op,
        Operation::PageRank { .. } | Operation::KCore | Operation::Degrees
    )
}

/// The rows to show: the first `--top N` (10 on a terminal, every row when piped), or
/// the `N` largest values of a measure.
fn shown<'o>(op: &Operation, output: &'o Output, nodes: u32, req: &Request) -> Shown<'o> {
    let rows = nodes as usize;
    let piped = Style::stdout().piped && !req.common.json;
    let n = req.top.unwrap_or(if piped { rows } else { SHOWN });
    match (req.top, output.columns.first()) {
        (Some(_), Some((_, column))) if ranked(op) => Shown {
            columns: &output.columns,
            rows: table::top(column, rows, n),
            ranked: true,
        },
        _ => Shown {
            columns: &output.columns,
            rows: (0..nodes).take(n).collect(),
            ranked: false,
        },
    }
}

/// The facts above the table, as data (`summary` of the result document).
fn summary(op: &Operation, output: &Output) -> Option<Map<String, Value>> {
    let Some((_, Column::U32 { values, null })) = output.columns.first() else {
        return None;
    };
    let values = values.as_slice()?;
    let mut s = Map::new();
    match op {
        Operation::Components => {
            let components = values.iter().max().map_or(0, |&m| u64::from(m) + 1);
            let largest = values.iter().filter(|&&c| c == 0).count() as u64;
            s.insert("components".into(), components.into());
            s.insert("largest".into(), largest.into());
        }
        Operation::KCore => {
            s.insert(
                "max_core".into(),
                values.iter().max().copied().unwrap_or(0).into(),
            );
        }
        Operation::Bfs { .. } => {
            let reached = values.iter().filter(|&&d| Some(d) != *null);
            s.insert("reached".into(), (reached.clone().count() as u64).into());
            s.insert(
                "max_distance".into(),
                reached.max().copied().unwrap_or(0).into(),
            );
        }
        _ => return None,
    }
    Some(s)
}

/// The summary for a person, above the table on a terminal.
fn summary_text(op: &Operation, output: &Output, style: Style) -> String {
    let Some(s) = summary(op, output) else {
        return String::new();
    };
    let n = |k: &str| s.get(k).and_then(Value::as_u64).unwrap_or(0);
    let dot = style.dot();
    let line = match op {
        Operation::Components => format!(
            "{}{dot}the largest has {}",
            counted(n("components"), "component", "components"),
            counted(n("largest"), "node", "nodes")
        ),
        Operation::KCore => format!("largest core number {}", count(n("max_core"))),
        Operation::Bfs { .. } => format!(
            "{} reached{dot}the farthest {} away",
            counted(n("reached"), "node", "nodes"),
            counted(n("max_distance"), "hop", "hops")
        ),
        _ => return String::new(),
    };
    format!("{line}\n\n")
}

/// The detail of the phase line (§ 4.1): one or two exact facts.
fn detail(op: &Operation, output: &Output) -> String {
    let n = |k: &str| {
        summary(op, output)
            .and_then(|s| s.get(k).and_then(Value::as_u64))
            .unwrap_or(0)
    };
    match op {
        Operation::PageRank { .. } => output.iterations.map_or_else(String::new, |i| {
            counted(u64::from(i), "iteration", "iterations")
        }),
        Operation::Components => counted(n("components"), "component", "components"),
        Operation::KCore => format!("largest core {}", count(n("max_core"))),
        Operation::Degrees => String::new(),
        Operation::Bfs { .. } => format!("{} reached", counted(n("reached"), "node", "nodes")),
    }
}

/// The dim lines under a table on a terminal, on standard error (M-10, § 6.3).
fn footer(
    req: &Request,
    op: &Operation,
    shown: &Shown<'_>,
    nodes: u32,
    cached: bool,
    stored: bool,
) {
    let style = Style::stderr();
    let dot = style.dot();
    let mut text = format!(
        "\n{} of {} shown",
        count(shown.rows.len() as u64),
        counted(u64::from(nodes), "row", "rows")
    );
    let export = format!("export with --out {}.parquet", op.name());
    if stored {
        let name = req
            .graph
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let how = if cached { " (cached)" } else { "" };
        text.push_str(&format!("\nfull result: {name}{how}{dot}{export}"));
    } else {
        text.push_str(&format!("\n{export}"));
    }
    let _ = writeln!(io::stderr(), "{}", style.dim(&text));
}

/// The result document (`urn:everygraph:schema:result:1`, `docs/api-sketch.md` § 5.3).
#[expect(clippy::too_many_arguments, reason = "the facts of one document")]
fn document<S: ByteSource>(
    req: &Request,
    op: &Operation,
    output: &Output,
    shown: &Shown<'_>,
    nodes: &Nodes<'_, S>,
    stored_in: Option<String>,
    cached: bool,
    directed: bool,
) -> Result<Value, everygraph_ingest::ids::DictionaryError> {
    let rows = shown.json_rows(nodes)?;
    let returned = rows.len();
    let parameters: Map<String, Value> = op
        .parameters()
        .into_iter()
        .map(|(k, v)| {
            let value = v.parse::<f64>().ok().and_then(serde_json::Number::from_f64);
            (k.to_owned(), value.map_or(Value::String(v), Value::Number))
        })
        .collect();
    let mut meta = json!({
        "operation": op.name(),
        "parameters": parameters,
        "deterministic": true,
        "version": op.version(),
        "direction": if directed { "out" } else { "ignored" },
        "cached": cached,
    });
    if let Some(iterations) = output.iterations {
        meta["iterations"] = iterations.into();
    }
    if let Some(converged) = output.converged {
        meta["converged"] = converged.into();
    }
    Ok(json!({
        "schema": 1,
        "result": op.name(),
        "graph": req.graph.display().to_string(),
        "view": null,
        "summary": summary(op, output),
        "columns": shown.names(),
        "rows": rows,
        "rows_returned": returned,
        "rows_total": nodes.dictionary().len(),
        "stored_in": stored_in,
        "cached": cached,
        "truncated": null,
        "meta": meta,
    }))
}
