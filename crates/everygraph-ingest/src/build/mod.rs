// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The graph build (issue #35): a text edge list to the sections of a `.eg` graph, in a
//! memory budget, with the same bytes at any thread count and any budget.
//!
//! # Passes
//!
//! 1. **Keys.** The identifier dictionary reads every block ([`crate::ids`]); invalid
//!    rows are counted and refused here.
//! 2. **Items.** Each row becomes its items: one per loop row kept, one per other row
//!    on a directed graph, one in each direction on an undirected one
//!    (`docs/spec/graph-semantics.md` § 2.3). This pass counts the items of each node.
//! 3. **Buckets.** Buckets are ranges of at most 2^16 source nodes, cut at exact
//!    boundaries from those counts (module `layout`). The longest prefix of buckets that
//!    fits the budget is filled in memory by `par::scatter`; the items of the other
//!    buckets go to spill files as 6-byte records (module `fill`). The rows are parsed again
//!    for this pass, twice when something is in memory (the scatter counts first).
//! 4. **Lists.** Bucket after bucket, in node order, each node's items are sorted; equal
//!    targets are one pair, with their number of items as its events; the list is
//!    encoded (module `finish`). Records go straight to the data section; the index and the
//!    `.events` section (ADR 0028) are written at the end.
//!
//! When everything fits, nothing is written but the output: no spill.
//!
//! Property columns are not carried: each entry's multiplicity, kept in module
//! `finish`, gives its pair's range of events.
//!
//! # Determinism
//!
//! Every boundary that depends on the budget (buckets, rounds, what spills) or on the
//! schedule (the order of spill chunks) is followed by a sort of each list, whose key is
//! total; the scatter is stable; the dictionary's form depends on its keys alone. So
//! the sections are a function of the input and the options: a budget gives these
//! bytes or a refusal, never other bytes (DESIGN D14).
//!
//! # The input
//!
//! Each block's identifiers are digested the first time it is read, by the
//! dictionary, and compared every later time: an input that changes or is cut short
//! between passes is refused ([`BuildError::InputChanged`]), never built from two
//! files.

mod estimate;
mod fill;
mod finish;
mod layout;
mod lists;
mod rows;
mod transpose;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io;
use std::num::NonZeroUsize;

use everygraph_core::{
    ByteSink, ByteSource, InsufficientBudget, Memory, MemoryError, MemoryKind, Reservation, widen,
};
use everygraph_format::adjacency::{AdjacencyEvents, AdjacencyIndex, EncodeError, FormatError};
use everygraph_format::container::{Container, ContainerError, Entry, Kind, Provenance, Writer};

use crate::ids::{
    self, Dictionary, DictionaryError, IdOrder, InvalidKeys, Scratch, build_dictionary,
};
use crate::text::{IdType, Layout as TextLayout, OnInvalid, Tally, TextError, TextReader};

pub use estimate::{Estimate, Sample, estimate};
use lists::{Context, Names, Written, fill_lists, write_lists};
use rows::{Rows, TextKeys};
pub use transpose::add_in_adjacency;

pub(crate) const ONE: NonZeroUsize = NonZeroUsize::MIN;

/// The out-adjacency's sections.
pub const OUT_DATA: &str = "adjacency.out.data";
/// The index of the out-adjacency.
pub const OUT_INDEX: &str = "adjacency.out.index";
/// The `.events` section of the out-adjacency, when some pair has more than one event.
pub const OUT_EVENTS: &str = "adjacency.out.events";
/// The data of the in-adjacency of a directed graph: the transpose of the out-adjacency
/// (DESIGN D6), derived from it.
pub const IN_DATA: &str = "adjacency.in.data";
/// The index of the in-adjacency.
pub const IN_INDEX: &str = "adjacency.in.index";
/// The `.events` section of the in-adjacency: present exactly when the out-adjacency
/// has one.
pub const IN_EVENTS: &str = "adjacency.in.events";
/// The provenance record of the build.
pub const PROVENANCE: &str = "provenance/ingest";

const OUT: Names = Names {
    data: OUT_DATA,
    index: OUT_INDEX,
    events: OUT_EVENTS,
};

/// What happens to a row from a node to itself (`docs/spec/graph-semantics.md` § 2.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelfLoops {
    /// Dropped and counted (the default, A3).
    #[default]
    Drop,
    /// Kept: a pair of the node with itself.
    Keep,
}

/// What happens to a row whose pair was seen before (`graph-semantics.md` § 2.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Duplicates {
    /// Each row is an event of its pair (the default, A1).
    #[default]
    Events,
    /// The pair keeps its first row; the others are dropped and counted.
    Collapse,
    /// The rows are merged into the pair's one edge.
    Sum,
}

/// The options that are part of the graph's identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphOptions {
    /// Directed, or undirected (each row is the unordered pair of its ends).
    pub directed: bool,
    /// Loop rows.
    pub self_loops: SelfLoops,
    /// Repeated rows.
    pub duplicates: Duplicates,
}

/// Where the direction of a graph came from (`docs/spec/graph-semantics.md` § 2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectedFrom {
    /// The caller said.
    Caller,
    /// The source and target columns are named `from`/`to` or `source`/`target`.
    ColumnNames,
    /// Nothing said: undirected.
    Default,
}

/// The direction of a graph read with `layout`: what the caller asked, else directed
/// exactly when the source and target columns are named like a direction (`from`/`to`,
/// `source`/`target`, in any case).
#[must_use]
pub fn infer_direction(layout: &TextLayout, asked: Option<bool>) -> (bool, DirectedFrom) {
    if let Some(directed) = asked {
        return (directed, DirectedFrom::Caller);
    }
    let name = |i| {
        layout
            .header
            .as_ref()
            .and_then(|h| h.get(i))
            .map(|n: &String| n.to_ascii_lowercase())
    };
    match (
        name(layout.source).as_deref(),
        name(layout.target).as_deref(),
    ) {
        (Some("from"), Some("to")) | (Some("source"), Some("target")) => {
            (true, DirectedFrom::ColumnNames)
        }
        _ => (false, DirectedFrom::Default),
    }
}

/// Options of [`build`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct BuildOptions {
    /// The graph's options.
    pub graph: GraphOptions,
    /// How the identifier dictionary is built.
    pub ids: ids::BuildOptions,
    /// The most blocks the parallel facade runs at once: the size of the thread pool.
    /// Buffers held per block in flight are reserved for this many. A larger pool
    /// than this makes the process hold more than the budget says.
    pub threads: NonZeroUsize,
    /// On a directed graph, write the in-adjacency too (the default).
    pub in_adjacency: bool,
}

impl BuildOptions {
    /// These graph options, the default dictionary, and `threads`.
    #[must_use]
    pub fn new(graph: GraphOptions, threads: NonZeroUsize) -> Self {
        Self {
            graph,
            ids: ids::BuildOptions::default(),
            threads,
            in_adjacency: true,
        }
    }
}

/// Sizes that tests shrink to reach many buckets, rounds and segments with small
/// inputs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tuning {
    /// `log2` of the rows of a segment of the bucket columns.
    pub(crate) segment_bits: u32,
    /// The most nodes of a bucket: at most 2^16, for 16-bit local sources.
    pub(crate) bucket_nodes: u32,
    /// Items a block gathers before it writes them to the spill files.
    pub(crate) spill_batch: usize,
    /// The most items held in memory, whatever the budget allows.
    pub(crate) resident_items: u64,
    /// Whether the mapped rows may be kept between passes.
    pub(crate) cache_rows: bool,
    /// Entries of the out-lists the transposition reads at least per block.
    pub(crate) transpose_block: u64,
}

const TUNING: Tuning = Tuning {
    segment_bits: 28,
    bucket_nodes: 1 << 16,
    spill_batch: 1 << 16,
    resident_items: u64::MAX,
    cache_rows: true,
    transpose_block: 1 << 18,
};

/// The phases of a build (`docs/ux-contract.md` § 4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// The input read once, for the identifier dictionary. Counts blocks.
    Reading,
    /// The dictionary built. No count.
    Identifiers,
    /// The items, the buckets and the lists. Counts rounds.
    Building,
    /// The written file read back and checked ([`check`]). Counts sections.
    Checking,
}

impl Phase {
    /// The name printed in progress lines.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Reading => "reading",
            Self::Identifiers => "identifiers",
            Self::Building => "building",
            Self::Checking => "checking",
        }
    }
}

/// Where a build says how far it is. Called from any thread.
pub trait Progress: Sync {
    /// `phase` starts; `total` is its count when known.
    fn begin(&self, phase: Phase, total: Option<u64>) {
        let _ = (phase, total);
    }

    /// `done` more of `phase`'s count are done.
    fn advance(&self, phase: Phase, done: u64) {
        let _ = (phase, done);
    }

    /// `phase` ended, having made `made`: rows read, identifiers, buckets, sections
    /// checked.
    fn end(&self, phase: Phase, made: u64) {
        let _ = (phase, made);
    }
}

/// A [`Progress`] that ignores everything.
#[derive(Clone, Copy, Debug, Default)]
pub struct Silent;

impl Progress for Silent {}

/// How a build ran (`docs/ux-contract.md` § 3.2, `regime.name`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Regime {
    /// Every item was held in memory: nothing was written but the output.
    InMemory,
    /// The items of some buckets went to spill files.
    OnDisk,
}

/// What a build read and wrote: the ingest report of `docs/spec/graph-semantics.md`
/// § 2.8, and how the build ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Report {
    /// Rows read.
    pub rows: u64,
    /// Rows that could not be read, or whose identifier does not fit the column's kind.
    pub rejected: u64,
    /// Loop rows dropped.
    pub dropped_self_loops: u64,
    /// Repeated rows dropped (`duplicates="collapse"`).
    pub dropped_duplicates: u64,
    /// Repeated rows merged into their pair (`duplicates="sum"`).
    pub merged: u64,
    /// Events stored.
    pub edges: u64,
    /// Loop rows stored.
    pub self_loops: u64,
    /// Rows stored as events that repeat a pair (`duplicates="events"`).
    pub repeated: u64,
    /// Pairs, kept loops included.
    pub pairs: u64,
    /// Nodes.
    pub nodes: u64,
    /// Nodes with no stored edge.
    pub isolates: u64,
    /// Entries of the out-adjacency.
    pub entries: u64,
    /// Stored events of the out-adjacency (its `.events` section's count).
    pub events: u64,
    /// Bytes of the `.events` section; 0 when there is none.
    pub events_bytes: u64,
    /// Whether the in-adjacency was written.
    pub in_adjacency: bool,
    /// In memory, or with spills.
    pub regime: Regime,
    /// Buckets the items were cut into.
    pub buckets: usize,
    /// Bytes written to spill files.
    pub spilled_bytes: u64,
}

impl Report {
    /// The counts under the keys of `docs/spec/graph-semantics.md` § 2.8, as stored in
    /// the provenance.
    #[must_use]
    pub fn counts(&self) -> [(&'static str, u64); 11] {
        [
            ("counts.rows", self.rows),
            ("dropped.rejected", self.rejected),
            ("dropped.self_loops", self.dropped_self_loops),
            ("dropped.duplicates", self.dropped_duplicates),
            ("counts.merged", self.merged),
            ("counts.edges", self.edges),
            ("counts.self_loops", self.self_loops),
            ("counts.repeated", self.repeated),
            ("counts.pairs", self.pairs),
            ("counts.nodes", self.nodes),
            ("counts.isolates", self.isolates),
        ]
    }
}

/// Why a build failed. Nothing it wrote is a graph: the sink holds no commit.
#[derive(Debug)]
#[non_exhaustive]
pub enum BuildError {
    /// The input could not be read, or has invalid rows.
    Text(TextError),
    /// Identifiers that do not fit the kind of their column, which makes their rows
    /// invalid (`design/DECISIONS.md`, "Identifier columns").
    InvalidKeys(InvalidKeys),
    /// The identifier dictionary failed.
    Dictionary(DictionaryError),
    /// The budget cannot hold a step: refused before it allocates.
    Budget(InsufficientBudget),
    /// Memory that fits the budget could not be allocated, or cannot be addressed.
    Memory(MemoryError),
    /// A compressed input: read it through [`TextReader::decompress_into`] first.
    Compressed,
    /// The input changed while it was read.
    InputChanged,
    /// The graph exceeds a limit of the format.
    TooLarge(&'static str),
    /// The scratch storage failed.
    Scratch(io::Error),
    /// The output container failed.
    Container(ContainerError),
    /// The encoder refused its input (an internal error).
    Encode(EncodeError),
    /// A section read back is not valid ([`check`]).
    Format(FormatError),
    /// The graph is undirected: it has no in-adjacency.
    Undirected,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(e) => e.fmt(f),
            Self::InvalidKeys(keys) => write!(
                f,
                "{} identifiers do not fit the kind of their column",
                keys.count
            ),
            Self::Dictionary(e) => e.fmt(f),
            Self::Budget(e) => e.fmt(f),
            Self::Memory(e) => e.fmt(f),
            Self::Compressed => f.write_str("a compressed input must be decompressed first"),
            Self::InputChanged => f.write_str("the input changed while it was read"),
            Self::TooLarge(what) => write!(f, "the graph is too large: {what}"),
            Self::Scratch(e) => write!(f, "temporary storage failed: {e}"),
            Self::Container(e) => e.fmt(f),
            Self::Encode(e) => e.fmt(f),
            Self::Format(e) => e.fmt(f),
            Self::Undirected => f.write_str("an undirected graph has no in-adjacency"),
        }
    }
}

impl Error for BuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Text(e) => Some(e),
            Self::Dictionary(e) => Some(e),
            Self::Budget(e) => Some(e),
            Self::Memory(e) => Some(e),
            Self::Scratch(e) => Some(e),
            Self::Container(e) => Some(e),
            Self::Encode(e) => Some(e),
            Self::Format(e) => Some(e),
            _ => None,
        }
    }
}

impl From<TextError> for BuildError {
    fn from(e: TextError) -> Self {
        Self::Text(e)
    }
}

impl From<DictionaryError> for BuildError {
    fn from(e: DictionaryError) -> Self {
        match e {
            DictionaryError::Budget(b) => Self::Budget(b),
            DictionaryError::Input(inner) if inner.is::<rows::Changed>() => Self::InputChanged,
            DictionaryError::Input(inner) => match inner.downcast::<TextError>() {
                Ok(text) => Self::Text(*text),
                Err(other) => Self::Dictionary(DictionaryError::Input(other)),
            },
            other => Self::Dictionary(other),
        }
    }
}

impl From<MemoryError> for BuildError {
    fn from(e: MemoryError) -> Self {
        match e {
            MemoryError::Budget(b) => Self::Budget(b),
            other => Self::Memory(other),
        }
    }
}

impl From<InsufficientBudget> for BuildError {
    fn from(e: InsufficientBudget) -> Self {
        Self::Budget(e)
    }
}

impl From<io::Error> for BuildError {
    fn from(e: io::Error) -> Self {
        Self::Scratch(e)
    }
}

impl From<ContainerError> for BuildError {
    fn from(e: ContainerError) -> Self {
        Self::Container(e)
    }
}

impl From<EncodeError> for BuildError {
    fn from(e: EncodeError) -> Self {
        match e {
            EncodeError::OutOfMemory { bytes } => Self::Memory(MemoryError::AllocationFailed {
                purpose: "adjacency encoder",
                bytes,
            }),
            EncodeError::Budget(refused) => Self::Budget(refused),
            other => Self::Encode(other),
        }
    }
}

impl From<FormatError> for BuildError {
    fn from(e: FormatError) -> Self {
        Self::Format(e)
    }
}

impl From<everygraph_core::ReadError> for BuildError {
    fn from(e: everygraph_core::ReadError) -> Self {
        Self::Text(TextError::Read(e))
    }
}

/// A build whose sections are written but not committed: the provenance record, which
/// holds the report, comes last ([`commit`](Self::commit)).
#[derive(Debug)]
pub struct Unfinished<K: ByteSink> {
    writer: Writer<K>,
    report: Report,
    parameters: BTreeMap<String, String>,
}

impl<K: ByteSink> Unfinished<K> {
    /// What the build read and wrote.
    #[must_use]
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// Adds `provenance` with the build's parameters and report (its `operation` is
    /// set to `ingest`), points every section at it, and commits. Returns the sink,
    /// which then holds a graph.
    ///
    /// # Errors
    ///
    /// The container failed; the sink then holds no commit.
    pub fn commit(self, mut provenance: Provenance) -> Result<(K, Report), BuildError> {
        let Self {
            mut writer,
            report,
            parameters,
        } = self;
        provenance.operation = "ingest".into();
        provenance.parameters.extend(parameters);
        for (key, value) in report.counts() {
            provenance.parameters.insert(key.into(), value.to_string());
        }
        let names: Vec<String> = writer.entries().map(|e| e.name.to_string()).collect();
        writer.add_provenance(PROVENANCE, &provenance)?;
        for name in names {
            writer.set_provenance(&name, Some(PROVENANCE))?;
        }
        writer.commit()?;
        Ok((writer.into_sink(), report))
    }
}

/// Builds the graph of the plain-text edge list `reader` into the container `out`,
/// within the budget of `memory`, with `scratch` for what does not fit. The sections
/// are written; [`Unfinished::commit`] adds the provenance and commits.
///
/// # Errors
///
/// [`BuildError::Compressed`] for a compressed input; [`BuildError::Text`] when the
/// input cannot be read or has invalid rows under [`OnInvalid::Refuse`];
/// [`BuildError::InvalidKeys`] likewise for identifiers; [`BuildError::Budget`] when a
/// step cannot fit the budget, before it allocates; the other variants for failures of
/// the scratch, the output or the input during the build.
pub fn build<S, Sc, K>(
    reader: &TextReader<S>,
    options: &BuildOptions,
    memory: &Memory,
    scratch: &Sc,
    out: K,
    progress: &dyn Progress,
) -> Result<Unfinished<K>, BuildError>
where
    S: ByteSource + Sync,
    Sc: Scratch,
    K: ByteSink + Sync,
{
    build_tuned(reader, options, memory, scratch, out, progress, &TUNING)
}

/// Memory kept free of every plan, for what no reservation holds (the allocator's own
/// rounding, thread stacks): a sixteenth of what is available.
const SLACK_SHIFT: u32 = 4;

fn build_tuned<S, Sc, K>(
    reader: &TextReader<S>,
    options: &BuildOptions,
    memory: &Memory,
    scratch: &Sc,
    out: K,
    progress: &dyn Progress,
    tuning: &Tuning,
) -> Result<Unfinished<K>, BuildError>
where
    S: ByteSource + Sync,
    Sc: Scratch,
    K: ByteSink + Sync,
{
    let blocks = reader.blocks()?.ok_or(BuildError::Compressed)?;
    let threads = options.threads.get();
    let graph = options.graph;

    progress.begin(Phase::Reading, Some(widen(blocks)));
    let fingerprints = rows::Fingerprints::new(blocks);
    let keys = TextKeys::new(reader, &fingerprints, progress);
    // A row is at least `0 0` and a line end, so the text holds at most `bytes / 2 + 1`
    // keys: integer keys far above that can only take the hashed form, at any budget.
    let mut ids_options = options.ids;
    ids_options.max_keys = ids_options.max_keys.or(Some(reader.source().len() / 2 + 1));
    let built = build_dictionary(&keys, &ids_options, memory.accountant(), scratch)?;
    if blocks == 0 {
        progress.end(Phase::Reading, 0);
        progress.begin(Phase::Identifiers, None);
    }
    let tallies = keys.into_tallies();
    let most_rows = tallies.iter().map(Tally::rows).max().unwrap_or(0);
    let summary = reader.summarize(tallies)?;
    if built.invalid.count > 0 && reader.on_invalid() == OnInvalid::Refuse {
        return Err(BuildError::InvalidKeys(built.invalid));
    }
    let dictionary: Dictionary<'_, Sc::Source> = built.dictionary;
    let nodes = dictionary.len();
    progress.end(Phase::Identifiers, u64::from(nodes));

    progress.begin(Phase::Building, None);
    // The mapped rows are kept for the next passes when they take at most an eighth of
    // what is left: they are freed before the lists are finished, but the buckets
    // filled in memory live beside them.
    let cache_bytes = summary.rows.saturating_mul(rows::CACHE_BYTES_PER_ROW);
    let cached_rows = (cache_bytes <= memory.available() / 8).then_some(summary.rows);
    let flight = rows::Flight {
        threads,
        blocks,
        block_bytes: widen(reader.block_bytes()),
        most_rows,
    };
    let cached_rows = cached_rows.filter(|_| tuning.cache_rows);
    let rows = Rows::new(
        reader,
        &dictionary,
        graph,
        &fingerprints,
        &flight,
        memory,
        cached_rows,
    )?;
    let cx = Context {
        graph,
        memory,
        scratch,
        tuning,
        threads,
        progress,
    };
    let filled = fill_lists(&rows, nodes, graph.directed, &cx)?;
    drop(rows);
    let (pass, isolates) = (filled.pass, filled.isolates);
    let mut spilled_bytes = filled.spilled_bytes;
    let (mut regime, buckets) = (filled.layout().regime(), filled.layout().buckets());
    let mut made = buckets;
    let mut writer = Writer::create(out)?;
    let Written {
        pairs,
        events_bytes,
    } = write_lists(filled, &mut writer, OUT, &cx)?;
    let in_adjacency = graph.directed && options.in_adjacency;
    if in_adjacency {
        let transposed = transpose::transpose_into(&mut writer, &cx)?;
        spilled_bytes += transposed.spilled_bytes;
        made += transposed.buckets;
        if !transposed.in_memory {
            regime = Regime::OnDisk;
        }
    }
    progress.end(Phase::Building, widen(made));
    copy_dictionary(&dictionary, &mut writer, memory)?;

    let events = graph.duplicates == Duplicates::Events;
    let rejected = summary.invalid_rows + pass.rejected;
    let (dropped_duplicates, merged, repeated) = match graph.duplicates {
        Duplicates::Events => (0, 0, pairs.repeats),
        Duplicates::Collapse => (pairs.repeats, 0, 0),
        Duplicates::Sum => (0, pairs.repeats, 0),
    };
    let report = Report {
        rows: pass.rows + summary.invalid_rows,
        rejected,
        dropped_self_loops: pass.dropped_loops,
        dropped_duplicates,
        merged,
        edges: pairs.pairs + repeated,
        self_loops: if events {
            pairs.loop_rows
        } else {
            pairs.loop_pairs
        },
        repeated,
        pairs: pairs.pairs,
        nodes: u64::from(nodes),
        isolates,
        entries: pairs.entries,
        events: pairs.events,
        events_bytes,
        in_adjacency,
        regime,
        buckets,
        spilled_bytes,
    };
    let mut parameters = parameters(&graph, reader.layout());
    let mut put = |key: &str, value: String| parameters.insert(key.to_owned(), value);
    put(
        "order",
        match dictionary.order() {
            IdOrder::SortedKey => "sorted-key",
            _ => "first-appearance",
        }
        .into(),
    );
    put("isolated_kept", report.isolates.to_string());
    put("isolated_dropped", "0".into());
    Ok(Unfinished {
        writer,
        report,
        parameters,
    })
}

/// The parameters that a build of the text read as `layout` with `graph` records in its
/// provenance, before what the build itself finds (graph order, isolated nodes): the
/// direction, self-loops, duplicates, the identifier type and the source and target
/// columns. Two builds with the same parameters from the same input make the same graph,
/// so a file whose provenance holds these is the cache of that build
/// (`docs/api-sketch.md` § 2.7).
#[must_use]
pub fn parameters(graph: &GraphOptions, layout: &TextLayout) -> BTreeMap<String, String> {
    let self_loops = match graph.self_loops {
        SelfLoops::Drop => "drop",
        SelfLoops::Keep => "keep",
    };
    let duplicates = match graph.duplicates {
        Duplicates::Events => "events",
        Duplicates::Collapse => "collapse",
        Duplicates::Sum => "sum",
    };
    let ids = match layout.ids {
        IdType::String => "string",
        IdType::Hex => "hex",
        _ => "integer",
    };
    [
        ("directed", graph.directed.to_string()),
        ("self_loops", self_loops.into()),
        ("duplicates", duplicates.into()),
        ("ids", ids.into()),
        ("source", layout.column_name(layout.source)),
        ("target", layout.column_name(layout.target)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

/// Copies the sections of the dictionary into the container, a reserved MiB at a time.
fn copy_dictionary<D: ByteSource, K: ByteSink>(
    dictionary: &Dictionary<'_, D>,
    writer: &mut Writer<K>,
    memory: &Memory,
) -> Result<(), BuildError> {
    const PIECE: usize = 64 << 10;
    let _reservation = memory.reserve("dictionary copy", MemoryKind::Anonymous, widen(PIECE))?;
    let mut buf = vec![0_u8; PIECE];
    for (name, source) in dictionary.sections() {
        let mut section = writer.begin_section(name, Kind::ID_DICTIONARY, None)?;
        let mut at = 0;
        while at < source.len() {
            let n = usize::try_from((source.len() - at).min(widen(PIECE))).expect("below PIECE");
            source.read_at(at, &mut buf[..n])?;
            section.write(&buf[..n])?;
            at += widen(n);
        }
        section.finish()?;
    }
    Ok(())
}

/// What [`check`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Checked {
    /// Sections whose bytes match their hash: all of them.
    pub sections: usize,
    /// Nodes of the out-adjacency and of the dictionary.
    pub nodes: u32,
    /// Entries of the out-adjacency.
    pub entries: u64,
    /// Stored events, when the graph has a `.events` section.
    pub events: Option<u64>,
    /// Whether the file holds the in-adjacency.
    pub in_adjacency: bool,
}

/// Reads a graph file back and checks it: every section against its hash, the index of
/// the out-adjacency, its `.events` section whole when the index announces one (and
/// none otherwise, under any name of kind 5), and the dictionary's sections, which
/// must have as many keys as the adjacency has nodes. An in-adjacency is checked the
/// same way, and must have the nodes, the entries and the events of the out-adjacency
/// (`docs/spec/eg-adjacency.md` § 5); that it is the transpose is for
/// `everygraph_format::verify`.
///
/// # Errors
///
/// [`BuildError::Container`] when the file or a section is damaged,
/// [`BuildError::Format`] for an invalid adjacency, [`BuildError::Dictionary`] for an
/// invalid dictionary, [`BuildError::Budget`] when the index does not fit `memory`.
pub fn check<S: ByteSource>(source: S, memory: &Memory) -> Result<Checked, BuildError> {
    let file = Container::open(source)?;
    file.verify_all()?;
    let stray = file.entries().iter().any(|e| {
        (e.kind == Kind::ADJACENCY_EVENTS) != (&*e.name == OUT_EVENTS || &*e.name == IN_EVENTS)
    });
    if stray {
        return Err(FormatError::Invalid(
            "a section of kind 5 that is not adjacency.out.events or adjacency.in.events",
        )
        .into());
    }
    let (nodes, entries, events) = check_adjacency(&file, OUT, memory)?;
    let in_adjacency = file.get(IN_INDEX).is_some() || file.get(IN_DATA).is_some();
    if in_adjacency && check_adjacency(&file, transpose::IN, memory)? != (nodes, entries, events) {
        return Err(FormatError::Invalid(
            "the in-adjacency differs from the out-adjacency in nodes, entries or events",
        )
        .into());
    }
    let dictionary = Dictionary::open(memory.accountant(), |name| file.section(name).ok())?;
    if dictionary.len() != nodes {
        return Err(
            DictionaryError::Corrupt("the dictionary and the adjacency differ in nodes").into(),
        );
    }
    Ok(Checked {
        sections: file.entries().len(),
        nodes,
        entries,
        events,
        in_adjacency,
    })
}

/// The nodes, entries and stored events (with a `.events` section) of the adjacency
/// `names`, whose index and `.events` section are checked.
fn check_adjacency<S: ByteSource>(
    file: &Container<S>,
    names: Names,
    memory: &Memory,
) -> Result<(u32, u64, Option<u64>), BuildError> {
    let mut held = Reservation::empty(memory, "checking", MemoryKind::Anonymous);
    let read = |name: &str, held: &mut Reservation| {
        read_section(file.source(), file.get(name), name, held)
    };
    let index = AdjacencyIndex::parse(&read(names.index, &mut held)?)?;
    let data = file
        .get(names.data)
        .ok_or_else(|| ContainerError::NoSuchSection(names.data.into()))?;
    if data.len != index.data_bytes() {
        return Err(FormatError::Length {
            expected: index.data_bytes(),
            actual: data.len,
        }
        .into());
    }
    let events = match (index.has_events(), file.get(names.events)) {
        (true, Some(_)) => {
            let bytes = read(names.events, &mut held)?;
            let events = AdjacencyEvents::parse(&bytes, &index)?;
            events.check()?;
            Some(events.events())
        }
        (false, None) => None,
        _ => {
            return Err(
                FormatError::Invalid("index flag bit 1 and the .events section disagree").into(),
            );
        }
    };
    Ok((index.nodes(), index.entries(), events))
}

/// Section `name` of the container in `source` (`entry` its entry) read whole into
/// memory, for which `held` grows by three times its length: the bytes and what a parser
/// builds beside them (an index's sequences, the copy of an `.events` sequence).
pub(crate) fn read_section<R: ByteSource>(
    source: &R,
    entry: Option<&Entry>,
    name: &str,
    held: &mut Reservation,
) -> Result<Vec<u8>, BuildError> {
    let entry = entry.ok_or_else(|| ContainerError::NoSuchSection(name.into()))?;
    let len = usize::try_from(entry.len).map_err(|_| BuildError::TooLarge("a section"))?;
    held.try_grow(entry.len.saturating_mul(3))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| MemoryError::AllocationFailed {
            purpose: "a section read whole",
            bytes: entry.len,
        })?;
    bytes.resize(len, 0);
    source
        .read_at(entry.offset, &mut bytes)
        .map_err(|e| BuildError::Container(ContainerError::Read(e)))?;
    Ok(bytes)
}
