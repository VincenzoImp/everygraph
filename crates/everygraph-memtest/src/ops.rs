// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The operations under test, each called as the command line and the Python module call
//! it, on an [`Input`] prepared from a [`Recipe`]: the files a user would have (the edge
//! list, the graph file) and the arrays a streamed run reads from.

use std::fmt::Debug;
use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use everygraph_core::{ByteSource, Memory, MemoryError, NodeId};
use everygraph_format::adjacency::Adjacency;
use everygraph_format::container::{Container, Provenance, compact};
use everygraph_format::verify::{Depth, verify};
use everygraph_ingest::build::{BuildError, BuildOptions, GraphOptions, Sample, Silent, estimate};
use everygraph_ingest::graph::{Graph, OpenError};
use everygraph_ingest::ids::{ColumnType, DictionaryError, Key, KeySource, build_dictionary};
use everygraph_ingest::native::{JobScratch, Output, WriteError, write};
use everygraph_ingest::text::{Delimiter, Header, TextOptions, TextReader};
use everygraph_io::{FileSink, FileSource, MmapSource};
use everygraph_kernels::components::{self, WccOptions};
use everygraph_kernels::degrees::{self, Loop};
use everygraph_kernels::edge_map::{Compressed, Error, Plan, Settings, Source, Stream};
use everygraph_kernels::kcore;
use everygraph_kernels::pagerank::{self, PageRank};
use everygraph_kernels::sssp::{Sssp, sssp};
use everygraph_kernels::traversal::{Bfs, bfs};

use crate::graphs::Recipe;

/// Threads of every operation, so that per-thread buffers are the same at every size.
pub const THREADS: NonZeroUsize = NonZeroUsize::new(4).unwrap();

/// Rows per chunk of the dictionary's key source.
const KEY_CHUNK: usize = 1 << 12;

const EDGES: &str = "edges.txt";
const GRAPH: &str = "graph.eg";
const TARGETS: &str = "targets.u32";

/// What an operation reads: files in a directory, opened, and the arrays of a streamed
/// run. Made outside the measurement.
#[derive(Debug)]
pub struct Input {
    /// The directory of the files.
    pub dir: PathBuf,
    /// Nodes of the graph.
    pub nodes: u32,
    text: MmapSource,
    graph: MmapSource,
    targets: FileSource,
    offsets: Vec<u64>,
    costs: Vec<f64>,
    lines: Vec<usize>,
}

impl Input {
    /// Writes the files of `recipe` into `dir` (made if needed) and opens them.
    ///
    /// # Panics
    ///
    /// If a file cannot be written or the graph cannot be built: the test's own setup.
    pub fn prepare(recipe: &Recipe, dir: &Path) -> Self {
        std::fs::create_dir_all(dir).expect("the input directory");
        std::fs::write(dir.join(EDGES), recipe.text()).expect("the edge list");
        let text = MmapSource::open(dir.join(EDGES)).expect("the edge list");
        let scratch = dir.join("scratch");
        std::fs::create_dir_all(&scratch).expect("the scratch directory");
        ingest_into(&text, &Memory::unlimited(), &scratch, &dir.join(GRAPH)).expect("the build");
        std::fs::remove_dir(&scratch).expect("an empty scratch directory");
        let graph = MmapSource::open(dir.join(GRAPH)).expect("the graph file");
        let g = Graph::open(&graph, &Memory::unlimited()).expect("the graph");
        let data = g.data().expect("the data part");
        let lists = Adjacency::new(g.index(), data.as_slice().expect("mapped")).expect("lists");
        let mut out = BufWriter::new(File::create(dir.join(TARGETS)).expect("the targets"));
        let mut list = Vec::new();
        for v in 0..g.nodes() {
            for t in lists.neighbors(NodeId::new(v), &mut list).expect("a list") {
                out.write_all(&t.to_le_bytes()).expect("the targets");
            }
        }
        out.flush().expect("the targets");
        Self::reopen(dir)
    }

    /// Opens the files [`prepare`](Self::prepare) wrote into `dir`.
    ///
    /// # Panics
    ///
    /// If its files cannot be read.
    pub fn reopen(dir: &Path) -> Self {
        let text = MmapSource::open(dir.join(EDGES)).expect("the edge list");
        let graph = MmapSource::open(dir.join(GRAPH)).expect("the graph file");
        let index = Graph::open(&graph, &Memory::unlimited()).expect("the graph");
        let mut offsets = vec![0];
        offsets.extend(index.index().degrees(0..index.nodes()).scan(0, |end, d| {
            *end += u64::from(d);
            Some(*end)
        }));
        drop(index);
        let entries = offsets.last().copied().unwrap_or(0);
        Self {
            dir: dir.to_path_buf(),
            nodes: u32::try_from(offsets.len() - 1).expect("u32 nodes"),
            lines: line_starts(text.as_slice().unwrap_or_default()),
            text,
            graph,
            targets: FileSource::open(dir.join(TARGETS)).expect("the stream's targets"),
            offsets,
            costs: (0..entries).map(|e| 1.0 + (e % 7) as f64).collect(),
        }
    }

    /// Heap bytes the input holds: what a child process holds before the operation.
    #[must_use]
    pub fn heap_bytes(&self) -> u64 {
        ((self.offsets.len() + self.costs.len() + self.lines.len()) * 8) as u64
    }
}

/// Where each chunk of [`KEY_CHUNK`] lines starts, and the end.
fn line_starts(text: &[u8]) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        text.iter()
            .enumerate()
            .filter(|&(_, &b)| b == b'\n')
            .map(|(i, _)| i + 1)
            .skip(KEY_CHUNK - 1)
            .step_by(KEY_CHUNK)
            .filter(|&i| i < text.len()),
    );
    starts.push(text.len());
    starts
}

/// The text options of every build here: tab-separated, no header.
fn text_options() -> TextOptions {
    TextOptions {
        delimiter: Delimiter::Byte(b'\t'),
        header: Header::Absent,
        ..TextOptions::default()
    }
}

/// Builds the graph of the edge list `text` into the file `out`, as `everygraph ingest`
/// does.
fn ingest_into<S: ByteSource + Sync>(
    text: S,
    memory: &Memory,
    scratch: &Path,
    out: &Path,
) -> Result<(), Failure> {
    let reader = TextReader::open(text, &text_options()).map_err(other)?;
    let options = BuildOptions::new(GraphOptions::default(), THREADS);
    let job = JobScratch::create(scratch, "memtest").map_err(other)?;
    let provenance = Provenance {
        engine: "everygraph-memtest".into(),
        ..Provenance::default()
    };
    match write(
        &reader,
        &options,
        memory,
        &job,
        Output::File(out),
        provenance,
        &Silent,
        &|| true,
    ) {
        Ok(_) => Ok(()),
        Err(WriteError::Build(e)) => Err(Failure::of(build_refused(&e), e)),
        Err(e) => Err(other(e)),
    }
}

/// Why an operation did not complete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The budget was too small: the variant of the operation's error type that says
    /// so (DESIGN D2), written out.
    Budget(String),
    /// Any other error, written out.
    Other(String),
}

impl Failure {
    fn of(budget: bool, e: impl Debug) -> Self {
        if budget {
            Self::Budget(format!("{e:?}"))
        } else {
            Self::Other(format!("{e:?}"))
        }
    }
}

fn other(e: impl Debug) -> Failure {
    Failure::Other(format!("{e:?}"))
}

fn build_refused(e: &BuildError) -> bool {
    match e {
        BuildError::Budget(_) => true,
        BuildError::Memory(e) => memory_refused(e),
        BuildError::Dictionary(e) => matches!(e, DictionaryError::Budget(_)),
        _ => false,
    }
}

fn memory_refused(e: &MemoryError) -> bool {
    matches!(e, MemoryError::Budget(_))
}

fn kernel(e: Error) -> Failure {
    Failure::of(matches!(&e, Error::Memory(m) if memory_refused(m)), e)
}

/// The identifiers of the edge list, both ends of each row, as the dictionary reads them.
struct Keys<'a> {
    input: &'a Input,
}

impl KeySource for Keys<'_> {
    fn column(&self) -> ColumnType {
        ColumnType::Utf8
    }

    fn chunks(&self) -> usize {
        self.input.lines.len() - 1
    }

    fn read_chunk(
        &self,
        chunk: usize,
        f: &mut dyn FnMut(u64, Key<'_>) -> Result<(), DictionaryError>,
    ) -> Result<(), DictionaryError> {
        let text = self.input.text.as_slice().unwrap_or_default();
        let lines = &text[self.input.lines[chunk]..self.input.lines[chunk + 1]];
        let first = (chunk * KEY_CHUNK * 2) as u64;
        let keys = lines
            .split(|&b| b == b'\n')
            .flat_map(|line| line.split(|&b| b == b'\t'))
            .filter(|key| !key.is_empty());
        for (i, key) in keys.enumerate() {
            f(first + i as u64, Key::Bytes(key))?;
        }
        Ok(())
    }
}

/// How a kernel reads its lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Regime {
    /// The compressed lists of the graph file, mapped (the in-memory mode, D11).
    Mapped,
    /// Raw lists read from a file in batches (the semi-external mode).
    Streamed,
}

/// A kernel of `everygraph-kernels`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kernel {
    /// `PageRank`, f64.
    PageRank,
    /// BFS from node 0, with parents.
    Bfs,
    /// BFS from node 0, hops only.
    Hops,
    /// SSSP from node 0.
    Sssp,
    /// Weakly connected components, numbered.
    Wcc,
    /// Core numbers.
    KCore,
    /// Degrees.
    Degrees,
    /// Leiden, deterministic mode.
    Leiden,
}

/// An operation under test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `everygraph ingest`: the edge list to a graph file, checked.
    Ingest,
    /// The identifier dictionary of the edge list's keys (#34).
    Dictionary,
    /// A kernel over the graph file.
    Kernel(Kernel, Regime),
    /// `everygraph verify --deep`.
    Verify,
    /// `everygraph compact`: a copy of the graph file with its live sections.
    Compact,
}

/// Untracked heap that an operation holds today, outside the ledger, in some of its
/// phases, with the issue that removes it. The suite exempts those phases only, and
/// checks that they still hold it, so that the entry goes with the fix.
#[derive(Clone, Copy, Debug)]
pub struct Known {
    /// The operation.
    pub op: Op,
    /// The phases exempt: those whose purpose, the last reservation before them
    /// ([`untracked_by_phase`](crate::counting::untracked_by_phase)), starts with one of
    /// these.
    pub phases: &'static [&'static str],
    /// The most anonymous bytes these phases can add to the control group's peak, which
    /// cannot be split by phase.
    pub bound: u64,
    /// Whether a budget below the declared minimum is refused only after the work has
    /// begun, instead of before it.
    pub refuses_late: bool,
    /// What is held, and the issue.
    pub why: &'static str,
}

/// The known findings.
pub const KNOWN: [Known; 1] = [Known {
    op: Op::Ingest,
    // The first pass is the dictionary's read of the text: its phases, and the time
    // before its first reservation. The dictionary alone is checked as `Op::Dictionary`.
    phases: &["(before any reservation)", "id dictionary"],
    // One parsed block per thread: 2 × 4 MiB of text and 24 bytes for each of at most
    // 4 MiB / 4 rows (a row is at least `0 0` and a line end; `build/rows.rs`, `Flight`).
    bound: THREADS.get() as u64 * ((8 << 20) + (1 << 20) * 24),
    refuses_late: true,
    why: "the first pass holds each text block's parsed rows outside the ledger, before the \
          dictionary's first reservation and during its read, and a budget below the build's own estimate is \
          refused only after the first pass (plan/issues/m3-28-ingest-first-pass.md)",
}];

/// The known finding of `op`, if there is one.
#[must_use]
pub fn known(op: Op) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.op == op)
}

/// Every operation, each kernel in both regimes.
pub const OPS: [Op; 20] = {
    use Kernel::{Bfs, Degrees, Hops, KCore, Leiden, PageRank, Sssp, Wcc};
    use Regime::{Mapped, Streamed};
    [
        Op::Ingest,
        Op::Dictionary,
        Op::Kernel(PageRank, Mapped),
        Op::Kernel(Bfs, Mapped),
        Op::Kernel(Hops, Mapped),
        Op::Kernel(Sssp, Mapped),
        Op::Kernel(Wcc, Mapped),
        Op::Kernel(KCore, Mapped),
        Op::Kernel(Degrees, Mapped),
        Op::Kernel(Leiden, Mapped),
        Op::Kernel(PageRank, Streamed),
        Op::Kernel(Bfs, Streamed),
        Op::Kernel(Hops, Streamed),
        Op::Kernel(Sssp, Streamed),
        Op::Kernel(Wcc, Streamed),
        Op::Kernel(KCore, Streamed),
        Op::Kernel(Degrees, Streamed),
        Op::Kernel(Leiden, Streamed),
        Op::Verify,
        Op::Compact,
    ]
};

impl Op {
    /// A name for reports and for the command line of a child process.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Kernel(k, r) => format!("{k:?}/{r:?}").to_lowercase(),
            other => format!("{other:?}").to_lowercase(),
        }
    }

    /// The operation of [`name`](Self::name).
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        OPS.into_iter().find(|op| op.name() == name)
    }

    /// The smallest budget the engine declares for the operation on `input` before it
    /// runs, below which it refuses before any work: the planner's (`m3-02-planner`),
    /// and today the build's own estimate (`everygraph_ingest::build::estimate`). `None`
    /// where nothing is declared yet.
    ///
    /// # Panics
    ///
    /// If the edge list cannot be read.
    #[must_use]
    pub fn declared_minimum(self, input: &Input) -> Option<u64> {
        match self {
            Self::Ingest => {
                let reader = TextReader::open(&input.text, &text_options()).expect("the edge list");
                let sample = Sample::of(&reader).expect("a sample")?;
                let options = BuildOptions::new(GraphOptions::default(), THREADS);
                Some(estimate(&sample, &options, u64::MAX).smallest_budget)
            }
            _ => None,
        }
    }

    /// Whether the operation takes a budget. One that does not is still checked: all
    /// its heap is untracked, which must stay bounded.
    #[must_use]
    pub const fn budgeted(self) -> bool {
        !matches!(self, Self::Compact)
    }

    /// Runs the operation on `input` within `memory`, with `scratch` (an empty
    /// directory) for its files. A file it leaves there is a leak.
    ///
    /// # Errors
    ///
    /// The operation's error: [`Failure::Budget`] for the error that says the budget is
    /// too small, and for a report of `verify` that leaves a check undone for want of
    /// memory.
    pub fn run(self, input: &Input, memory: &Memory, scratch: &Path) -> Result<(), Failure> {
        match self {
            Self::Ingest => {
                let out = scratch.join(GRAPH);
                let built = ingest_into(&input.text, memory, scratch, &out);
                let _ = std::fs::remove_file(out);
                built
            }
            Self::Dictionary => {
                let options = everygraph_ingest::ids::BuildOptions::default();
                let job = JobScratch::create(scratch, "dictionary").map_err(other)?;
                match build_dictionary(&Keys { input }, &options, memory.accountant(), &job) {
                    Ok(built) if !built.dictionary.is_empty() => Ok(()),
                    Ok(built) => Err(other(format!("no keys, {} invalid", built.invalid.count))),
                    Err(e) => Err(Failure::of(matches!(e, DictionaryError::Budget(_)), e)),
                }
            }
            Self::Kernel(kernel, Regime::Mapped) => {
                let g = Graph::open(&input.graph, memory)
                    .map_err(|e| Failure::of(matches!(e, OpenError::Budget(_)), e))?;
                let data = g.data().map_err(other)?;
                let lists = Compressed::mapped(g.index(), &data).map_err(other)?;
                kernel.run(&lists, input, memory)
            }
            Self::Kernel(kernel, Regime::Streamed) => {
                let lists = Stream::new(&input.offsets, &input.targets).map_err(other)?;
                kernel.run(&lists, input, memory)
            }
            Self::Verify => {
                let report = verify(&input.graph, Depth::Deep, memory).map_err(other)?;
                if !report.is_intact() {
                    Err(other(report.problems))
                } else if report.unchecked.is_empty() {
                    Ok(())
                } else {
                    let short = |u: &String| u.contains("not enough memory");
                    Err(Failure::of(
                        report.unchecked.iter().all(short),
                        report.unchecked,
                    ))
                }
            }
            Self::Compact => {
                let from = Container::open(&input.graph).map_err(other)?;
                let out = scratch.join(GRAPH);
                let sink = FileSink::create(&out).map_err(other)?;
                let done = compact(&from, sink).map(drop).map_err(other);
                let _ = std::fs::remove_file(out);
                done
            }
        }
    }
}

impl Kernel {
    fn run<S: Source>(self, lists: &S, input: &Input, memory: &Memory) -> Result<(), Failure> {
        if self == Self::Leiden {
            let options = everygraph_community::Options::default();
            return everygraph_community::leiden(lists, memory, Settings::default(), &options)
                .map(drop)
                .map_err(kernel);
        }
        Plan::with(lists, memory, Settings::default(), |plan| match self {
            Self::PageRank => {
                let strength = pagerank::strength(&plan, None, false, memory)?;
                let s = strength.as_slice().ok_or(Error::Invalid("one segment"))?;
                let graph = pagerank::Graph {
                    lists: &plan,
                    weights: None,
                    directed: false,
                };
                pagerank::pagerank::<_, f64>(&graph, s, &PageRank::default(), memory).map(drop)
            }
            Self::Bfs => {
                let p = Bfs {
                    max_hops: None,
                    parents: true,
                };
                bfs(&plan, Some(&plan), 0, &p, memory).map(drop)
            }
            Self::Hops => bfs(&plan, Some(&plan), 0, &Bfs::default(), memory).map(drop),
            Self::Sssp => sssp(&plan, &input.costs, 0, &Sssp::default(), memory).map(drop),
            Self::Wcc => {
                let options = WccOptions {
                    symmetric: true,
                    sample: true,
                };
                let (labels, held) = components::wcc(&plan, memory, options)?;
                let numbered = components::number(&labels, memory)?;
                drop((labels, held, numbered));
                Ok(())
            }
            Self::KCore => kcore::kcore(&plan, memory).map(drop),
            Self::Degrees => degrees::degrees(&plan, memory, Loop::Twice).map(drop),
            Self::Leiden => Err(Error::Invalid("Leiden runs without a plan")),
        })
        .map_err(kernel)
    }
}
