// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph run NAME GRAPH` (`docs/api-sketch.md` § 5.1–5.3): a whole-graph
//! algorithm. A result on the whole graph is looked up in the graph file before it is
//! computed, and kept there after (§ 2.8), under the identity of the graph it was
//! computed on; `bfs` is never kept. The table goes to standard output — aligned on a
//! terminal, tab-separated when piped — or, with `--out`, to a file; with `--json`, the
//! result document.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use everygraph_core::{ByteSource, Memory, MemoryError, par};
use everygraph_format::container::{GraphIdentity, Provenance, Writer};
use everygraph_ingest::graph::Graph;
use everygraph_ingest::ids::Key;
use everygraph_io::{FileSink, MmapSource};
use everygraph_kernels::edge_map::{Compressed, Error, Plan, Settings};
use everygraph_kernels::pagerank::PageRank;
use everygraph_kernels::run::{self, Column, Operation, Output, Precision, RunError, VERSION};
use serde_json::{Value, json};

use crate::args::{self, Common};
use crate::failure::{self, Failure};
use crate::graph;
use crate::ingest::is_graph;
use crate::machine::{self, Machine};
use crate::progress::{self, Lines, now_ns};
use crate::render::{Style, count, counted, size};
use crate::table::{self, Nodes, Rows, SHOWN, Shown};

const HELP: &str = "\
Run a whole-graph algorithm.

Usage
  everygraph run NAME GRAPH [NODE] [OPTIONS]

Names
  pagerank              the PageRank of each node
  components            the weakly connected component of each node, numbered
                        from 0 by decreasing size
  kcore                 the core number of each node
  degrees               the edges at each node, a kept loop counted twice; on a
                        directed graph, the edges out of it
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

Short forms: -o for --out, -m for --memory, -t for --threads.
";

/// Attempts to take the writer of a graph file that another process holds, with
/// waits doubling from the first.
const STORE_ATTEMPTS: u32 = 6;
const FIRST_WAIT: Duration = Duration::from_millis(50);

/// `everygraph run NAME …`.
#[derive(Debug, clap::Args)]
#[command(override_help = HELP)]
pub(crate) struct Args {
    #[command(subcommand)]
    algorithm: Algorithm,
}

/// The names `run` takes.
pub(crate) const NAMES: [&str; 5] = run::NAMES;

#[derive(Debug, clap::Subcommand)]
#[command(override_help = HELP, allow_negative_numbers = true)]
enum Algorithm {
    #[command(override_help = HELP, allow_negative_numbers = true)]
    Pagerank {
        #[command(flatten)]
        on: On,
        #[arg(long, value_name = "X", value_parser = args::probability,
            default_value_t = PageRank::default().damping)]
        damping: f64,
        #[arg(long, value_name = "X", value_parser = args::tolerance,
            default_value_t = PageRank::default().tol)]
        tol: f64,
        #[arg(long, value_name = "N", value_parser = args::count,
            default_value_t = PageRank::default().max_iter)]
        max_iter: u32,
        #[arg(long, value_enum, default_value = "full")]
        precision: Bits,
    },
    #[command(override_help = HELP, allow_negative_numbers = true)]
    Components {
        #[command(flatten)]
        on: On,
    },
    #[command(override_help = HELP, allow_negative_numbers = true)]
    Kcore {
        #[command(flatten)]
        on: On,
    },
    #[command(override_help = HELP, allow_negative_numbers = true)]
    Degrees {
        #[command(flatten)]
        on: On,
    },
    #[command(override_help = HELP, allow_negative_numbers = true)]
    Bfs {
        #[command(flatten)]
        on: On,
        #[arg(value_name = "NODE", allow_hyphen_values = true)]
        node: String,
        #[arg(long, value_name = "N", value_parser = args::count)]
        max_hops: Option<u32>,
        #[arg(long, value_enum, default_value = "out")]
        direction: Direction,
    },
}

/// What every algorithm takes.
#[derive(Debug, clap::Args)]
struct On {
    #[arg(value_name = "GRAPH")]
    graph: PathBuf,
    #[arg(long, value_name = "N", value_parser = args::rows)]
    top: Option<usize>,
    #[command(flatten)]
    common: Common,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Bits {
    Full,
    Low,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Direction {
    Out,
    In,
    Both,
}

/// Runs the command; returns the exit status.
pub(crate) fn main(args: Args) -> u8 {
    let (op, on, node) = match args.algorithm {
        Algorithm::Pagerank {
            on,
            damping,
            tol,
            max_iter,
            precision,
        } => {
            let params = PageRank {
                damping,
                tol,
                max_iter,
                personalize: None,
            };
            let precision = match precision {
                Bits::Full => Precision::Full,
                Bits::Low => Precision::Low,
            };
            (Operation::PageRank { params, precision }, on, None)
        }
        Algorithm::Components { on } => (Operation::Components, on, None),
        Algorithm::Kcore { on } => (Operation::KCore, on, None),
        Algorithm::Degrees { on } => (Operation::Degrees, on, None),
        Algorithm::Bfs {
            on,
            direction: Direction::In | Direction::Both,
            ..
        } => {
            return unsupported(
                "--direction in and both cannot be used yet.".into(),
                "They need the in-adjacency, which no graph file holds yet.",
            )
            .report(on.common.json);
        }
        Algorithm::Bfs {
            on, node, max_hops, ..
        } => (
            Operation::Bfs {
                source: 0,
                max_hops,
            },
            on,
            Some(node),
        ),
    };
    execute(op, &on, node.as_deref()).unwrap_or_else(|failure| failure.report(on.common.json))
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

fn execute(mut op: Operation, on: &On, node: Option<&str>) -> Result<u8, Failure> {
    let started = Instant::now();
    let started_at = now_ns();
    let common = &on.common;
    let machine = Machine::probe();
    let budget = machine.budget(common)?;
    let threads = machine.threads(common);
    if let Some(out) = &common.out {
        table::format_of(out)?;
        if out.exists() && !common.overwrite {
            return Err(failure::output_exists(out, is_graph(out)));
        }
    }
    let memory = machine::memory(budget);
    let graph = graph::open(&on.graph, &memory, budget)?;
    let corrupt = |e: everygraph_ingest::ids::DictionaryError| failure::corrupt(&on.graph, &e);
    let nodes = Nodes::new(graph.dictionary(&memory).map_err(corrupt)?);
    if let (Operation::Bfs { source, .. }, Some(node)) = (&mut op, node) {
        *source = lookup(&nodes, node, &on.graph)?;
    }
    let directed = graph.directed();
    // The graph this run computes on: a file replaced meanwhile holds another one.
    let identity = graph.container().graph_identity();
    let fail = |e| kernel_failure(e, &op, &on.graph, budget, machine.total());
    let lines = Lines::new(Style::stderr().progress(common.plain()));
    let name = op.name();
    lines.start(name);
    let cached =
        run::stored(graph.container(), &op, directed, graph.nodes(), &memory).map_err(fail)?;
    let is_cached = cached.is_some();
    let output = match cached {
        Some(output) => {
            lines.seconds(name);
            lines.kept(name, &Summary::of(&op, &output).facts(Style::stderr()));
            output
        }
        None => {
            let output = par::with_threads(threads, || compute(&graph, &op, &memory))
                .map_err(|e| failure::internal(&format!("Cannot start the threads: {e}.")))?
                .map_err(fail)?;
            lines.done_if_long(name, &Summary::of(&op, &output).facts(Style::stderr()));
            output
        }
    };
    let mut stored_in = None;
    if is_cached {
        stored_in = Some(on.graph.display().to_string());
    } else if op.stored() {
        let about = Provenance {
            engine: format!("everygraph {}", env!("CARGO_PKG_VERSION")),
            started: started_at,
            finished: now_ns(),
            ..Provenance::default()
        };
        match store(&on.graph, &op, identity, &about, &output) {
            Ok(()) => stored_in = Some(on.graph.display().to_string()),
            Err(why) => failure::warning(
                &format!("The result could not be kept in {}.", on.graph.display()),
                &why,
                &[],
                "It is in the output, and is computed again next time.",
            ),
        }
    }
    let shown = shown(&op, &output, graph.nodes(), on, &memory)?;
    if let Some(out) = &common.out {
        let bytes = table::write_result(out, &nodes, &memory, &output.columns, &corrupt)?;
        stored_in = Some(out.display().to_string());
        if !common.json {
            let rows = counted(u64::from(graph.nodes()), "row", "rows");
            progress::made(out, &[size(bytes), rows], started.elapsed().as_secs_f64());
        }
    }
    let stdout = Style::stdout();
    if common.json {
        let done = Done {
            op: &op,
            node,
            output: &output,
            shown: &shown,
            stored_in,
            cached: is_cached,
            directed,
        };
        println!("{}", done.document(on, &nodes, &memory, &corrupt)?);
    } else if common.out.is_none() {
        if stdout.terminal {
            let facts = Summary::of(&op, &output).facts(stdout);
            let mut text = if facts.is_empty() || matches!(op, Operation::PageRank { .. }) {
                String::new()
            } else {
                format!("{facts}\n\n")
            };
            text.push_str(&shown.aligned(&nodes, &memory, stdout, &corrupt)?);
            let _ = io::stdout().write_all(text.as_bytes());
            footer(
                on,
                &op,
                &shown,
                graph.nodes(),
                is_cached,
                stored_in.is_some(),
            );
        } else {
            let mut out = io::BufWriter::new(io::stdout().lock());
            shown.piped(&nodes, &memory, &mut out, &corrupt)?;
        }
    }
    Ok(0)
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

/// Commits `output` to the graph file as the result of `op` on the graph `identity`;
/// the reason it could not. Another process that holds the file is waited for, a
/// few seconds at most.
fn store(
    path: &Path,
    op: &Operation,
    identity: GraphIdentity,
    about: &Provenance,
    output: &Output,
) -> Result<(), String> {
    let mut wait = FIRST_WAIT;
    let mut attempt = 1;
    let sink = loop {
        match FileSink::open(path) {
            Err(e) if e.kind() == io::ErrorKind::ResourceBusy && attempt < STORE_ATTEMPTS => {
                std::thread::sleep(wait);
                wait *= 2;
                attempt += 1;
            }
            other => break other.map_err(|e| failure::reason(&e))?,
        }
    };
    let mut writer = Writer::update(sink).map_err(|e| e.to_string())?;
    if writer.graph_identity() != identity {
        return Err(format!(
            "{} was replaced by another graph while the result was computed.",
            path.display()
        ));
    }
    run::store(&mut writer, op, identity, about, output).map_err(|e| e.to_string())?;
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
        RunError::Threads(e) => failure::internal(&format!("Cannot start the threads: {e}.")),
        RunError::Refused(r) => Failure::new(r.error, r.message, failure::NO_MEMORY)
            .why(vec![r.why])
            .ways(
                r.ways_forward
                    .into_iter()
                    .map(|w| (w.command, w.note))
                    .collect(),
            )
            .on_existing_file(),
        RunError::Kernel(Error::Memory(MemoryError::Budget(refused))) => {
            failure::memory(op.name(), budget, machine, &refused).on_existing_file()
        }
        RunError::Kernel(Error::Argument(why)) => Failure::new(
            "UsageError",
            format!("{} cannot use its parameters ({why}).", op.name()),
            failure::USAGE,
        )
        .ways(vec![("everygraph run --help".into(), String::new())])
        .on_existing_file(),
        RunError::Kernel(other) => failure::corrupt(graph, &other),
        RunError::Stored(e) => failure::corrupt(graph, &e),
    }
}

/// The rows to show: the first `--top N` (10 on a terminal, every row when piped), or
/// the `N` largest values of a measure.
fn shown<'o>(
    op: &Operation,
    output: &'o Output,
    nodes: u32,
    on: &On,
    memory: &Memory,
) -> Result<Shown<'o>, Failure> {
    let rows = nodes as usize;
    let piped = !Style::stdout().terminal && !on.common.json;
    let n = on.top.unwrap_or(if piped { rows } else { SHOWN });
    let ranked = matches!(
        op,
        Operation::PageRank { .. } | Operation::KCore | Operation::Degrees
    );
    let rows = match (on.top, output.columns.first()) {
        (Some(_), Some((_, column))) if ranked => table::top(column, rows, n, memory)
            .map_err(|e| failure::memory("--top", memory.available(), None, &e))?,
        _ => Rows::First(n.min(rows)),
    };
    Ok(Shown {
        columns: &output.columns,
        ranked: matches!(rows, Rows::Chosen { .. }),
        rows,
    })
}

/// The facts above the table (`summary` of the result document).
enum Summary {
    None,
    Iterations(u32),
    Components { count: u64, largest: u64 },
    Core { max: u32 },
    Reached { nodes: u64, farthest: u32 },
}

impl Summary {
    fn of(op: &Operation, output: &Output) -> Self {
        let values = match output.columns.first() {
            Some((_, Column::U32 { values, null, .. })) => values
                .as_slice()
                .map(|v| v.iter().copied().filter(|x| Some(*x) != *null)),
            _ => None,
        };
        match (op, values) {
            (Operation::PageRank { .. }, _) => {
                output.iterations.map_or(Self::None, Self::Iterations)
            }
            (Operation::Components, Some(v)) => {
                let (mut count, mut largest) = (0, 0);
                for c in v {
                    count = count.max(u64::from(c) + 1);
                    largest += u64::from(c == 0);
                }
                Self::Components { count, largest }
            }
            (Operation::KCore, Some(v)) => Self::Core {
                max: v.max().unwrap_or(0),
            },
            (Operation::Bfs { .. }, Some(v)) => {
                let (mut nodes, mut farthest) = (0, 0);
                for d in v {
                    nodes += 1;
                    farthest = farthest.max(d);
                }
                Self::Reached { nodes, farthest }
            }
            _ => Self::None,
        }
    }

    fn json(&self) -> Value {
        match self {
            Self::None | Self::Iterations(_) => Value::Null,
            Self::Components { count, largest } => json!({"components": count, "largest": largest}),
            Self::Core { max } => json!({"max_core": max}),
            Self::Reached { nodes, farthest } => {
                json!({"reached": nodes, "max_distance": farthest})
            }
        }
    }

    /// For a person: the summary line, and the detail of the phase line (§ 4.1).
    fn facts(&self, style: Style) -> String {
        let dot = style.dot();
        match self {
            Self::None => String::new(),
            Self::Iterations(i) => counted(u64::from(*i), "iteration", "iterations"),
            Self::Components { count, largest } => format!(
                "{}{dot}the largest has {}",
                counted(*count, "component", "components"),
                counted(*largest, "node", "nodes")
            ),
            Self::Core { max } => format!("largest core number {}", count(u64::from(*max))),
            Self::Reached { nodes, farthest } => format!(
                "{} reached{dot}the farthest {} away",
                counted(*nodes, "node", "nodes"),
                counted(u64::from(*farthest), "hop", "hops")
            ),
        }
    }
}

/// The dim lines under a table on a terminal, on standard error (M-10, § 6.3).
fn footer(on: &On, op: &Operation, shown: &Shown<'_>, nodes: u32, cached: bool, stored: bool) {
    let style = Style::stderr();
    let dot = style.dot();
    let mut text = format!(
        "\n{} of {} shown",
        count(shown.rows.len() as u64),
        counted(u64::from(nodes), "row", "rows")
    );
    let export = format!("export with --out {}.parquet", op.name());
    if stored {
        let name = on
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

/// What a run did, for its document.
struct Done<'a> {
    op: &'a Operation,
    /// The identifier `bfs` was given, as typed.
    node: Option<&'a str>,
    output: &'a Output,
    shown: &'a Shown<'a>,
    stored_in: Option<String>,
    cached: bool,
    directed: bool,
}

impl Done<'_> {
    /// The parameters as the user gave them, with their types (§ 5.3).
    fn parameters(&self) -> Value {
        match self.op {
            Operation::PageRank { params, precision } => json!({
                "damping": params.damping,
                "tol": params.tol,
                "max_iter": params.max_iter,
                "precision": match precision { Precision::Full => "full", Precision::Low => "low" },
            }),
            Operation::Bfs { max_hops, .. } => json!({"source": self.node, "max_hops": max_hops}),
            _ => json!({}),
        }
    }

    /// The result document (`urn:everygraph:schema:result:1`, `docs/api-sketch.md`
    /// § 5.3).
    fn document<S: ByteSource>(
        &self,
        on: &On,
        nodes: &Nodes<'_, S>,
        memory: &Memory,
        corrupt: &dyn Fn(everygraph_ingest::ids::DictionaryError) -> Failure,
    ) -> Result<Value, Failure> {
        let rows = self.shown.json_rows(nodes, memory, corrupt)?;
        let returned = rows.len();
        let mut meta = json!({
            "operation": self.op.name(),
            "parameters": self.parameters(),
            "deterministic": true,
            "version": VERSION,
            "direction": if self.directed { "out" } else { "ignored" },
            "cached": self.cached,
        });
        if let Some(iterations) = self.output.iterations {
            meta["iterations"] = iterations.into();
        }
        if let Some(converged) = self.output.converged {
            meta["converged"] = converged.into();
        }
        Ok(json!({
            "schema": 1,
            "result": self.op.name(),
            "graph": on.graph.display().to_string(),
            "view": null,
            "summary": Summary::of(self.op, self.output).json(),
            "columns": self.shown.names(),
            "rows": rows,
            "rows_returned": returned,
            "rows_total": nodes.dictionary().len(),
            "stored_in": self.stored_in,
            "cached": self.cached,
            "truncated": null,
            "meta": meta,
        }))
    }
}
