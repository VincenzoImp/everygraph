// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph export GRAPH --out FILE [--topology-only]` (`docs/api-sketch.md`
//! § 5.1): the graph's edges as a table, `source` and `target`, in the format of
//! `FILE`'s extension. One row per connected pair, in graph order; an undirected pair
//! once, from its smaller node. A graph file holds no property column yet, so every
//! export is of the structure alone; `--topology-only` says so, and the command prints
//! that the output is not anonymized (`design/DECISIONS.md`, "A structure-only
//! export"; `docs/research/README.md` H7).

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use everygraph_format::adjacency::{Adjacency, Batch};
use everygraph_io::table::TableWriter;
use serde_json::json;

use crate::args::{self as cli, Arg, Common};
use crate::failure::{self, Failure};
use crate::graph;
use crate::ingest::is_graph;
use crate::machine::{self, Machine};
use crate::progress::Lines;
use crate::render::{Style, counted, duration, size};
use crate::scratch;
use crate::table::{self, Nodes};

const HELP: &str = "\
Write the edges of a graph to a file, in the format of its extension.

Usage
  everygraph export GRAPH --out FILE [OPTIONS]

Options
  --out FILE            where the edges go: .parquet, .arrow, .csv or .json
  --topology-only       the edges alone, without any property column; the output
                        is not anonymized: structure alone can identify people
  --overwrite           replace FILE if it is in the way
  --json                print the file document, with nothing around it
  --progress plain      progress as one line per finished phase, never redrawn
  -h, --help            this text
";

/// Pairs per batch written.
const BATCH_ENTRIES: usize = 1 << 16;

/// Runs the command; returns the exit status.
pub(crate) fn main(raw: &[OsString]) -> u8 {
    let json = raw.iter().any(|a| a == "--json");
    match parse(raw).and_then(|p| {
        p.map_or(Ok(0), |(path, common, topology)| {
            run(&path, &common, topology)
        })
    }) {
        Ok(status) => status,
        Err(failure) => failure.report(json),
    }
}

fn parse(raw: &[OsString]) -> Result<Option<(PathBuf, Common, bool)>, Failure> {
    let mut it = cli::Args::new(raw, "export");
    let (mut path, mut common, mut topology) = (None, Common::default(), false);
    while let Some(arg) = it.next()? {
        match arg {
            Arg::Option(help) if cli::is_help(help) => {
                print!("{HELP}");
                return Ok(None);
            }
            Arg::Option("--topology-only") => topology = true,
            Arg::Option(option) => {
                if !common.take(option, &mut it)? {
                    return Err(it.unknown(option));
                }
            }
            Arg::Plain(_) if path.is_some() => {
                return Err(it.usage("export takes one GRAPH.".into()));
            }
            Arg::Plain(p) => path = Some(PathBuf::from(p)),
        }
    }
    let path = path.ok_or_else(|| it.usage("export needs a GRAPH.".into()))?;
    if common.out.is_none() {
        return Err(it.usage("export needs --out FILE: the format follows its extension.".into()));
    }
    Ok(Some((path, common, topology)))
}

fn run(path: &Path, common: &Common, topology: bool) -> Result<u8, Failure> {
    let started = Instant::now();
    let out = common.out.as_deref().unwrap_or(path);
    table::format_of(out)?;
    if out.exists() && !common.overwrite {
        return Err(failure::output_exists(out, is_graph(out)));
    }
    let machine = Machine::probe();
    let budget = machine.budget(common)?;
    let memory = machine::memory(budget);
    let graph = graph::open(path, &memory, budget)?;
    let nodes = Nodes::new(
        graph
            .dictionary(memory.accountant())
            .map_err(|e| failure::corrupt(path, &e))?,
    );
    if topology {
        let _ = writeln!(
            io::stderr(),
            "The output is not anonymized: the structure of a graph alone can identify people."
        );
    }
    let data = graph.data().map_err(|e| graph::container_error(path, &e))?;
    let bytes = everygraph_core::ByteSource::as_slice(&data)
        .ok_or_else(|| failure::internal("The adjacency data is not in memory."))?;
    let adjacency = Adjacency::new(graph.index(), bytes).map_err(|e| failure::corrupt(path, &e))?;
    let lines = Lines::new(Style::stderr().progress(common.plain));
    lines.start("writing");
    let mut rows = 0_u64;
    let schema = [
        ("source", nodes.file_type(false)),
        ("target", nodes.file_type(false)),
    ];
    let written = table::write(out, &schema, |writer| {
        rows = pairs(&adjacency, graph.directed(), &nodes, writer, path, out)?;
        Ok(())
    })?;
    lines.done_if_long("writing", &counted(rows, "row", "rows"));

    let seconds = started.elapsed().as_secs_f64();
    let style = Style::stderr();
    let dot = style.dot();
    let name = out
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let line = format!(
        "{} {}{dot}{}{dot}{}{dot}{}\n",
        style.ok(),
        style.bold(&name),
        size(written),
        counted(rows, "edge", "edges"),
        duration(seconds)
    );
    let _ = io::stderr().write_all(line.as_bytes());
    if common.json {
        let fingerprint = format!(
            "export\0{}\0{}\0{topology}",
            scratch::absolute(path).display(),
            scratch::absolute(out).display()
        );
        let doc = json!({
            "schema": 1,
            "job": scratch::job_id(&fingerprint),
            "operation": "export",
            "made": {"path": out.display().to_string(), "kind": "file", "bytes": written},
            "counts": {"rows": rows},
            "dropped": {},
            "kept": [],
            "elapsed_seconds": seconds,
        });
        println!("{doc}");
    }
    Ok(0)
}

/// Writes every pair of `adjacency` to `writer`; returns how many. On an undirected
/// graph each pair is in both lists and is written once, from its smaller node.
fn pairs<S: everygraph_core::ByteSource>(
    adjacency: &Adjacency<'_>,
    directed: bool,
    nodes: &Nodes<'_, S>,
    writer: &mut TableWriter,
    graph: &Path,
    out: &Path,
) -> Result<u64, Failure> {
    let n = adjacency.index().nodes();
    let mut batch = Batch::default();
    let (mut sources, mut targets) = (Vec::new(), Vec::new());
    let mut rows = 0;
    let mut next = 0;
    while next < n {
        next = adjacency
            .decode_nodes(next..n, BATCH_ENTRIES, &mut batch)
            .map_err(|e| failure::corrupt(graph, &e))?;
        sources.clear();
        targets.clear();
        for (u, list) in batch.iter() {
            for &v in list.iter().filter(|&&v| directed || u.get() <= v) {
                sources.push(Some(u.get()));
                targets.push(Some(v));
            }
        }
        let corrupt = |e| failure::corrupt(graph, &e);
        let s = nodes.ids(sources.iter().copied()).map_err(corrupt)?;
        let t = nodes.ids(targets.iter().copied()).map_err(corrupt)?;
        writer
            .write(&[s.values(), t.values()])
            .map_err(|e| failure::io("write", out, &e))?;
        rows += sources.len() as u64;
    }
    Ok(rows)
}
