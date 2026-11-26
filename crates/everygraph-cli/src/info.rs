// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph info GRAPH` (`docs/api-sketch.md` § 3.2, "What it prints", and § 5.1):
//! what the file holds and what was computed so far, on standard output (S-1). With
//! `--json`, the info document (`urn:everygraph:schema:info:1`, § 5.3), which also
//! lists every section.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use everygraph_core::ByteSource;
use everygraph_format::container::{Kind, ListedResult, Provenance};
use everygraph_ingest::build::PROVENANCE;
use everygraph_ingest::graph::Graph;
use everygraph_ingest::ids::KeyKind;
use everygraph_io::MmapSource;
use everygraph_kernels::run::Operation;
use serde_json::{Map, Value, json};

use crate::args::Common;
use crate::failure::Failure;
use crate::graph;
use crate::machine::{self, Machine};
use crate::render::{Style, age, count, date, duration, printable, size, timestamp};

const HELP: &str = "\
What a graph file holds and what was computed so far.

Usage
  everygraph info GRAPH [--json]

Options
  --json                print it as JSON, with nothing around it
  -h, --help            this text
";

/// What the command was asked.
#[derive(Debug, clap::Args)]
#[command(override_help = HELP)]
pub(crate) struct Args {
    #[arg(value_name = "GRAPH")]
    graph: PathBuf,
    #[arg(long)]
    json: bool,
}

/// Runs the command; returns the exit status.
pub(crate) fn main(args: &Args) -> u8 {
    run(&args.graph, args.json).unwrap_or_else(|failure| failure.report(args.json))
}

fn run(path: &Path, json: bool) -> Result<u8, Failure> {
    let budget = Machine::probe().budget(&Common::default())?;
    let memory = machine::memory(budget);
    let graph = graph::open(path, &memory, budget)?;
    let record = graph
        .container()
        .read_provenance(PROVENANCE)
        .map_err(|e| graph::container_error(path, &e))?;
    let results = graph
        .container()
        .results()
        .map_err(|e| graph::container_error(path, &e))?;
    let ids = graph.dictionary(&memory).ok().map(|d| d.kind());
    let info = Info {
        path,
        graph: &graph,
        record: &record,
        ids,
        results: &results,
        now: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_nanos()).ok()),
    };
    if json {
        println!("{}", info.document());
    } else {
        print!("{}", info.text(Style::stdout()));
    }
    Ok(0)
}

/// Why a stored result is not served any more.
fn stale(result: &ListedResult) -> Option<&'static str> {
    if !result.of_this_graph {
        return Some("graph");
    }
    let version = result.provenance.parameters.get("version");
    let current = Operation::version_of(&result.provenance.operation).map(|v| v.to_string());
    (version != current.as_ref()).then_some("version")
}

/// The seconds a record's run took, when it says.
fn seconds(started: Option<i64>, finished: Option<i64>) -> Option<f64> {
    let ns = finished?.checked_sub(started?)?;
    #[expect(clippy::cast_precision_loss, reason = "a printed duration")]
    let seconds = ns as f64 / 1e9;
    (seconds >= 0.0).then_some(seconds)
}

struct Info<'a> {
    path: &'a Path,
    graph: &'a Graph<MmapSource>,
    /// The record of the build.
    record: &'a Provenance,
    ids: Option<KeyKind>,
    results: &'a [ListedResult],
    now: Option<i64>,
}

impl Info<'_> {
    fn parameter(&self, key: &str) -> &str {
        self.record.parameters.get(key).map_or("", String::as_str)
    }

    fn edges_text(&self, style: Style) -> String {
        let dot = style.dot();
        let direction = if self.graph.directed() {
            "directed"
        } else {
            "undirected"
        };
        let duplicates = match self.parameter("duplicates") {
            "events" => "duplicates kept as events",
            "collapse" => "duplicates collapsed",
            "sum" => "duplicates summed",
            _ => "",
        };
        let loops = match self.parameter("self_loops") {
            "drop" => "self-loops dropped",
            "keep" => "self-loops kept",
            _ => "",
        };
        [direction, duplicates, loops]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(dot)
    }

    fn ids_text(&self) -> String {
        match self.ids {
            Some(KeyKind::Integers) => "integers".into(),
            Some(KeyKind::Hex { digits, .. }) => format!("hexadecimal ({digits} digits)"),
            Some(KeyKind::Uuid) => "UUIDs".into(),
            Some(KeyKind::Base58 { .. }) => "base58".into(),
            Some(KeyKind::Fixed { width }) => format!("binary ({width} bytes)"),
            Some(_) => "text".into(),
            None => "unknown".into(),
        }
    }

    fn input_name(&self) -> Option<String> {
        let input = self.record.inputs.first()?;
        let name = Path::new(&input.name)
            .file_name()
            .map_or_else(|| input.name.clone(), |n| n.to_string_lossy().into_owned());
        Some(printable(&name))
    }

    fn text(&self, style: Style) -> String {
        let dot = style.dot();
        let file = self.graph.container();
        let name = self
            .path
            .file_name()
            .map_or_else(String::new, |n| printable(&n.to_string_lossy()));
        let mut t = format!("{}{dot}{}", style.bold(&name), size(file.source().len()));
        if let Some(created) = self.record.finished {
            let _ = write!(t, "{dot}created {}", date(created));
            if let Some(input) = self.input_name() {
                let _ = write!(t, " from {input}");
            }
        }
        t.push_str("\n\n");
        let label = |name: &str| style.dim(&format!("{name:<10}"));
        let (nodes, edges) = (
            count(u64::from(self.graph.nodes())),
            count(self.graph.edges()),
        );
        let width = nodes.len().max(edges.len());
        let _ = writeln!(t, "  {}{nodes:>width$}", label("nodes"));
        let _ = writeln!(
            t,
            "  {}{edges:>width$}{dot}{}",
            label("edges"),
            self.edges_text(style)
        );
        let _ = writeln!(t, "  {}{}", label("ids"), self.ids_text());
        let reclaimable = file.reclaimable_bytes();
        let _ = write!(
            t,
            "  {}generation {}{dot}{} sections",
            label("file"),
            count(file.generation()),
            count(file.entries().len() as u64)
        );
        if reclaimable > 0 {
            let _ = write!(t, "{dot}{} reclaimable", size(reclaimable));
        }
        t.push_str("\n\n  computed so far\n");
        if self.results.is_empty() {
            t.push_str("    nothing yet\n");
        }
        let width = self
            .results
            .iter()
            .map(|r| r.provenance.operation.chars().count())
            .max()
            .unwrap_or(0)
            + 2;
        for r in self.results {
            let p = &r.provenance;
            let mut facts = vec![size(r.bytes)];
            facts.extend(seconds(p.started, p.finished).map(duration));
            if let Some(finished) = p.finished {
                facts.push(date(finished));
                if let Some(now) = self.now {
                    let ago = u64::try_from(now.saturating_sub(finished) / 1_000_000_000);
                    facts.extend(ago.ok().map(age));
                }
            }
            match stale(r) {
                Some("graph") => facts.push("stale: of an earlier graph".into()),
                Some(_) => facts.push("stale: of another version".into()),
                None => {}
            }
            let operation = printable(&p.operation);
            let _ = writeln!(t, "    {operation:<width$}{}", facts.join(dot));
        }
        if reclaimable > 0 {
            let next = format!("everygraph compact {}", self.path.display());
            let _ = write!(t, "\n  next: {}\n", style.bold(&next));
        }
        t
    }

    fn document(&self) -> Value {
        let file = self.graph.container();
        let record = self.record;
        let counts = |prefix: &str| -> Map<String, Value> {
            record
                .parameters
                .iter()
                .filter_map(|(k, v)| {
                    Some((
                        k.strip_prefix(prefix)?.to_owned(),
                        v.parse::<u64>().ok()?.into(),
                    ))
                })
                .collect()
        };
        let sections: Vec<Value> = file
            .entries()
            .iter()
            .map(|e| json!({"name": &*e.name, "kind": kind_name(e.kind), "bytes": e.len}))
            .collect();
        let results: Vec<Value> = self
            .results
            .iter()
            .map(|r| {
                let p = &r.provenance;
                json!({
                    "operation": p.operation,
                    "section": r.name,
                    "bytes": r.bytes,
                    "engine": p.engine,
                    "finished": p.finished.map(timestamp),
                    "seconds": seconds(p.started, p.finished),
                    "parameters": p.parameters,
                    "stale": stale(r),
                })
            })
            .collect();
        let inputs: Vec<Value> = record
            .inputs
            .iter()
            .map(|i| json!({"name": i.name, "bytes": i.bytes}))
            .collect();
        json!({
            "schema": 1,
            "info": self.path.display().to_string(),
            "bytes": file.source().len(),
            "generation": file.generation(),
            "graph": {
                "nodes": self.graph.nodes(),
                "edges": self.graph.edges(),
                "directed": self.graph.directed(),
                "self_loops": Some(self.parameter("self_loops")).filter(|p| !p.is_empty()),
                "duplicates": Some(self.parameter("duplicates")).filter(|p| !p.is_empty()),
                "ids": self.ids_text(),
            },
            "build": {
                "engine": record.engine,
                "finished": record.finished.map(timestamp),
                "seconds": seconds(record.started, record.finished),
                "inputs": inputs,
                "counts": counts("counts."),
                "dropped": counts("dropped."),
            },
            "reclaimable_bytes": file.reclaimable_bytes(),
            "sections": sections,
            "results": results,
        })
    }
}

/// The kind of a section, in words (`docs/spec/eg-container.md` § 5).
fn kind_name(kind: Kind) -> String {
    let name = match kind {
        Kind::PROVENANCE => "provenance",
        Kind::ADJACENCY_INDEX => "adjacency index",
        Kind::ADJACENCY_DATA => "adjacency data",
        Kind::ID_DICTIONARY => "identifier dictionary",
        Kind::ADJACENCY_EVENTS => "adjacency events",
        Kind::SEGMENTS => "segments",
        Kind::COLUMN => "column",
        Kind::COLUMN_LAYOUT => "column layout",
        Kind::COLUMN_STATS => "column statistics",
        Kind::RESULT => "result",
        other => return format!("kind {}", other.code()),
    };
    name.to_owned()
}
