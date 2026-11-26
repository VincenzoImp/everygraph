// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A graph for Python (`docs/api-sketch.md` § 3.2): a `.eg` file mapped by its stable
//! range, or the bytes of a graph built in memory, opened within a memory budget of its
//! own. Its whole-graph algorithms are planned and run with the interpreter released,
//! in one session per graph (`run::Session`), each within the budget of its plan; their
//! results stay with that budget until Python lets go of them.

use std::fs::File;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use arrow_array::ArrayRef;
use arrow_array::builder::{
    BinaryBuilder, FixedSizeBinaryBuilder, Int8Builder, Int16Builder, Int32Builder, Int64Builder,
    StringBuilder, UInt8Builder, UInt16Builder, UInt32Builder, UInt64Builder,
};
use arrow_schema::DataType;
use everygraph_core::ByteSource;
use everygraph_core::{Budget, Memory, MemorySource, NodeId};
use everygraph_format::container::Container;
use everygraph_ingest::graph::Graph as GraphFile;
use everygraph_ingest::ids::{ColumnType, Key, OwnedKey};
use everygraph_io::{FileSource, MmapSource};
use everygraph_kernels::pagerank::PageRank;
use everygraph_kernels::planner;
use everygraph_kernels::run::{Direction, Operation, Precision, Session, Weight};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::error::{self, Fail, Failure};
use crate::kernels::{self, Asked, Extra, Parts};
use crate::run::{Event, run};
use crate::source::Bytes;
use crate::table::{NULL, Table};

/// The memory budget of every call of the module: `limit` bytes, its large buffers on
/// pages of their own, given back when freed (`design/DECISIONS.md`, "Large buffers get
/// pages of their own").
pub(crate) fn budget(limit: u64) -> Arc<Budget> {
    Arc::new(Budget::new(limit).with_pages(everygraph_io::PAGES))
}

/// What a graph and its results share.
pub(crate) struct Shared {
    file: GraphFile<Bytes>,
    budget: Arc<Budget>,
    memory: Memory,
    path: Option<PathBuf>,
    ids: ColumnType,
    /// What the planner reads of the file, but whether it keeps a result.
    facts: planner::Graph,
    /// Taken by a call for its run, and given back after it.
    session: Mutex<Session>,
}

impl Shared {
    /// Opens the graph of `bytes` under `budget`.
    pub(crate) fn open(
        bytes: Bytes,
        path: Option<PathBuf>,
        budget: Arc<Budget>,
    ) -> Result<Self, Fail> {
        let limit = budget.limit();
        let memory = Memory::from(Arc::clone(&budget));
        let name = path.clone().unwrap_or_else(|| PathBuf::from("(in memory)"));
        let file = GraphFile::open(bytes, &memory).map_err(|e| error::open(&name, e, limit))?;
        let ids = file
            .dictionary(&memory)
            .map_err(|e| error::dictionary(e, limit))?
            .column();
        let shown = path.as_ref().map(|p| p.display().to_string());
        let bytes = file.container().source().len();
        let mut facts = planner::Graph {
            directed: file.directed(),
            ..planner::Graph::of(file.index(), shown, bytes, file.edges())
        };
        if let Some(inn) = file.in_index() {
            facts = facts.with_in(inn);
        }
        Ok(Self {
            file,
            budget,
            memory,
            path,
            ids,
            facts,
            session: Mutex::new(Session::new()),
        })
    }

    /// The graph file.
    pub(crate) fn file(&self) -> &GraphFile<Bytes> {
        &self.file
    }

    /// The budget the graph was opened with: its indexes, the events a call counts, and
    /// what its tables derive (`sizes`).
    pub(crate) fn memory(&self) -> &Memory {
        &self.memory
    }

    /// The limit of that budget.
    pub(crate) fn budget_bytes(&self) -> u64 {
        self.budget.limit()
    }

    /// The `.eg` file, or `None` for a graph in memory.
    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// What the planner reads of the file.
    pub(crate) fn facts(&self) -> &planner::Graph {
        &self.facts
    }

    /// Nodes, as a row count.
    pub(crate) fn nodes(&self) -> usize {
        self.file.nodes() as usize
    }

    /// The Arrow type of the identifiers: the type of the input column.
    pub(crate) fn id_type(&self) -> DataType {
        match self.ids {
            ColumnType::Int { bits: 8 } => DataType::Int8,
            ColumnType::Int { bits: 16 } => DataType::Int16,
            ColumnType::Int { bits: 32 } => DataType::Int32,
            ColumnType::Int { .. } => DataType::Int64,
            ColumnType::UInt { bits: 8 } => DataType::UInt8,
            ColumnType::UInt { bits: 16 } => DataType::UInt16,
            ColumnType::UInt { bits: 32 } => DataType::UInt32,
            ColumnType::UInt { .. } => DataType::UInt64,
            ColumnType::Binary => DataType::Binary,
            ColumnType::FixedBinary { width } => DataType::FixedSizeBinary(i32::from(width)),
            _ => DataType::Utf8,
        }
    }

    /// The identifiers of `ids` (dense ids; [`NULL`] is null), as an array of
    /// [`id_type`](Self::id_type), each appended as it is read back.
    pub(crate) fn keys(&self, ids: impl ExactSizeIterator<Item = u32>) -> Result<ArrayRef, String> {
        let dictionary = self
            .file
            .dictionary(&self.memory)
            .map_err(|e| e.to_string())?;
        let mut keys = ids.map(|id| match id {
            NULL => Ok(None),
            id => dictionary
                .key(NodeId::new(id))
                .map(Some)
                .map_err(|e| e.to_string()),
        });
        let narrow = || "an identifier wider than its column".to_owned();
        macro_rules! ints {
            ($builder:ty, $variant:ident) => {{
                let mut b = <$builder>::new();
                for key in &mut keys {
                    match key? {
                        Some(OwnedKey::$variant(v)) => {
                            b.append_value(v.try_into().map_err(|_| narrow())?);
                        }
                        _ => b.append_null(),
                    }
                }
                Arc::new(b.finish()) as ArrayRef
            }};
        }
        Ok(match self.id_type() {
            DataType::Int8 => ints!(Int8Builder, Int),
            DataType::Int16 => ints!(Int16Builder, Int),
            DataType::Int32 => ints!(Int32Builder, Int),
            DataType::Int64 => ints!(Int64Builder, Int),
            DataType::UInt8 => ints!(UInt8Builder, UInt),
            DataType::UInt16 => ints!(UInt16Builder, UInt),
            DataType::UInt32 => ints!(UInt32Builder, UInt),
            DataType::UInt64 => ints!(UInt64Builder, UInt),
            DataType::FixedSizeBinary(width) => {
                let mut b = FixedSizeBinaryBuilder::new(width);
                for key in &mut keys {
                    match key? {
                        Some(OwnedKey::Bytes(v)) => b.append_value(v).map_err(|e| e.to_string())?,
                        _ => b.append_null(),
                    }
                }
                Arc::new(b.finish())
            }
            DataType::Binary => {
                let mut b = BinaryBuilder::new();
                for key in &mut keys {
                    match key? {
                        Some(OwnedKey::Bytes(v)) => b.append_value(v),
                        _ => b.append_null(),
                    }
                }
                Arc::new(b.finish())
            }
            _ => {
                let mut b = StringBuilder::new();
                for key in &mut keys {
                    match key? {
                        Some(OwnedKey::Text(t)) => b.append_value(t),
                        Some(other) => b.append_value(other.to_string()),
                        None => b.append_null(),
                    }
                }
                Arc::new(b.finish())
            }
        })
    }

    /// The dense ids of `keys`, `None` for each unknown one.
    fn lookup(&self, keys: &[Bound<'_, PyAny>]) -> PyResult<Vec<Option<u32>>> {
        let dictionary = self
            .file
            .dictionary(&self.memory)
            .map_err(|e| error::dictionary(e, self.budget.limit()).into_err(keys[0].py()))?;
        keys.iter()
            .map(|key| {
                let int = if let Ok(v) = key.extract::<u64>() {
                    Some(dictionary.lookup(Key::UInt(v)))
                } else if let Ok(v) = key.extract::<i64>() {
                    Some(dictionary.lookup(Key::Int(v)))
                } else {
                    None
                };
                // An integer that a text column holds as its decimal text (`-1`).
                let found = if let Some(found) = int {
                    match found {
                        Ok(None) => dictionary.lookup(Key::Bytes(key.str()?.to_str()?.as_bytes())),
                        other => other,
                    }
                } else if let Ok(v) = key.extract::<&str>() {
                    dictionary.lookup(Key::Bytes(v.as_bytes()))
                } else {
                    dictionary.lookup(Key::Bytes(key.extract::<&[u8]>()?))
                };
                found
                    .map(|id| id.map(NodeId::get))
                    .map_err(|e| error::dictionary(e, self.budget.limit()).into_err(key.py()))
            })
            .collect()
    }
}

/// A graph.
#[pyclass(frozen, module = "everygraph._native")]
pub(crate) struct Graph(pub(crate) Arc<Shared>);

/// Opens the graph file at `path` with a budget of `memory` bytes.
#[pyfunction]
pub(crate) fn open(py: Python<'_>, path: PathBuf, memory: u64) -> PyResult<Graph> {
    let opened = py.detach(|| open_path(&path, budget(memory)));
    opened
        .map(|s| Graph(Arc::new(s)))
        .map_err(|e| e.into_err(py))
}

/// Opens the graph file at `path`, mapped by its stable range (`docs/spec/eg-container.md`
/// § 11), so that a commit by another writer never changes a mapped byte.
pub(crate) fn open_path(path: &Path, budget: Arc<Budget>) -> Result<Shared, Fail> {
    let os = |e| Fail::Os(e, path.display().to_string());
    let file = File::open(path).map_err(os)?;
    let source = FileSource::from_file(file.try_clone().map_err(os)?).map_err(os)?;
    let range = Container::open(source)
        .map_err(|e| error::open(path, e.into(), budget.limit()))?
        .stable_range();
    let map = MmapSource::from_file_range(&file, range).map_err(os)?;
    Shared::open(Bytes::File(map), Some(path.to_path_buf()), budget)
}

/// A graph of the bytes `bytes`, built in memory.
pub(crate) fn in_memory(bytes: Vec<u8>, budget: Arc<Budget>) -> Result<Shared, Fail> {
    Shared::open(Bytes::Memory(MemorySource::new(bytes)), None, budget)
}

/// How the package asks a call to run: the budget and its origin, the most threads and
/// their origin (`argument`, `graph`, `environment`, `default`), and whether only to plan.
#[derive(FromPyObject)]
pub(crate) struct Run(Option<u64>, String, Option<NonZeroUsize>, String, bool);

impl Shared {
    /// Plans `op` as `run` asks, sends the plan document to `on_event` (`plan`, its
    /// text), and returns the plan for a dry run, refuses one that does not fit, or runs
    /// it within its budget on its threads. Then `on_event` gets each phase as it begins
    /// (`begin`), the last as it ends (`end`), or a kept phase (`kept`), with the seconds
    /// since the start and the budget's peak. The result is the table, or `None` for a
    /// dry run, with what its `meta` gains: `plan` and `plan_text` always, and `unkept`,
    /// why a result could not be kept in the file.
    fn call<'py>(
        self: &Arc<Self>,
        py: Python<'py>,
        op: &Operation<'_>,
        run_as: &Run,
        on_event: &Bound<'py, PyAny>,
    ) -> PyResult<(Option<Table>, Bound<'py, PyDict>)> {
        let Run(memory, memory_from, threads, threads_from, dry_run) = run_as;
        let asked = Asked {
            memory: memory.map(|m| (m, kernels::origin(memory_from))),
            threads: threads.map(|t| (t, kernels::origin(threads_from))),
        };
        let parts = Parts::of(self, op).map_err(|e| e.into_err(py))?;
        let mut session =
            std::mem::take(&mut *self.session.lock().unwrap_or_else(PoisonError::into_inner));
        let give_back =
            |session| *self.session.lock().unwrap_or_else(PoisonError::into_inner) = session;
        let plan = match kernels::plan(self, op, &parts, &session, asked) {
            Ok(plan) => plan,
            Err(e) => {
                give_back(session);
                return Err(e.into_err(py));
            }
        };
        let document = plan.to_json();
        let info = PyDict::new(py);
        info.set_item("plan", &document)?;
        info.set_item("plan_text", plan.to_string())?;
        if let Err(e) = on_event.call1(("plan", &document, None::<u64>, 0.0, 0)) {
            give_back(session);
            return Err(e);
        }
        if *dry_run {
            give_back(session);
            return Ok((None, info));
        }
        if let Some(r) = plan.refusal() {
            give_back(session);
            return Err(Failure::new("Refusal")
                .text("document", r.to_json())
                .into_err(py));
        }
        let limit = kernels::budget_of(self, &plan, &parts, &session);
        let budget = budget(limit);
        let last = plan.phases().last().map_or(op.name(), |p| p.name);
        let out = run(py, NonZeroUsize::MIN, &budget, on_event, |events| {
            let memory = Memory::from(Arc::clone(&budget));
            if plan.kept() {
                events.send(Event::Kept(last));
            }
            let out = kernels::execute(self, &plan, &parts, &mut session, &memory, events);
            if out.is_ok() && !plan.kept() {
                events.send(Event::End(last, 0));
            }
            out
        });
        give_back(session);
        let (out, unkept) = out?.map_err(|e| e.into_err(py))?;
        if let Some(why) = unkept {
            info.set_item("unkept", why)?;
        }
        let mut table = Table::per_node(Arc::clone(self), out.columns);
        for (name, ids) in out.ids {
            table = table.with_ids(name, ids);
        }
        for (key, value) in out.extra {
            match value {
                Extra::Count(c) => info.set_item(key, c)?,
                Extra::Flag(b) => info.set_item(key, b)?,
            }
        }
        // What the plan counted outside the run's own budget: the graph's indexes and
        // events, and the session's raw copy.
        info.set_item("peak_memory_bytes", budget.peak() + (plan.budget() - limit))?;
        info.set_item("cached", plan.kept())?;
        Ok((Some(table), info))
    }
}

/// What a call returns: its table (`None` for a dry run) and what its `meta` gains.
type Called<'py> = PyResult<(Option<Table>, Bound<'py, PyDict>)>;

#[pymethods]
impl Graph {
    /// Nodes.
    #[getter]
    fn num_nodes(&self) -> u32 {
        self.0.file.nodes()
    }

    /// Stored edges: every event counts.
    #[getter]
    fn num_edges(&self) -> u64 {
        self.0.file.edges()
    }

    /// Whether the graph is directed.
    #[getter]
    fn directed(&self) -> bool {
        self.0.file.directed()
    }

    /// The weight column declared when the graph was read, if one was.
    #[getter]
    fn weight_column(&self) -> Option<&str> {
        self.0.file.weight()
    }

    /// The `.eg` file, or `None` for a graph in memory.
    #[getter]
    fn file(&self) -> Option<PathBuf> {
        self.0.path.clone()
    }

    /// Where the budget's buffers come from, and whether that is `everygraph_io::PAGES`.
    #[getter]
    fn pages(&self) -> (&'static str, bool) {
        let pages = everygraph_core::MemoryAccountant::pages(self.0.budget.as_ref());
        (pages.name(), pages.name() == everygraph_io::PAGES.name())
    }

    /// Bytes of the graph: the file, or what is held in memory.
    #[getter]
    fn bytes(&self) -> u64 {
        self.0.file.container().source().len()
    }

    /// One row per node, in graph order: `node`.
    fn nodes(&self) -> Table {
        Table::per_node(Arc::clone(&self.0), Vec::new())
    }

    /// The dense ids of `keys`, `None` for an unknown one.
    fn lookup(&self, keys: Vec<Bound<'_, PyAny>>) -> PyResult<Vec<Option<u32>>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        self.0.lookup(&keys)
    }

    /// `PageRank` (`everygraph.Graph.pagerank`); `personalize` holds dense ids. `edges`
    /// counts the edges of a pair, which only a graph with repeated pairs tells apart.
    #[pyo3(signature = (damping, tol, max_iter, edges, personalize, low, run, on_event))]
    #[expect(clippy::too_many_arguments, reason = "the keywords of Graph.pagerank")]
    fn pagerank<'py>(
        &self,
        py: Python<'py>,
        damping: f64,
        tol: f64,
        max_iter: u32,
        edges: bool,
        personalize: Option<Vec<u32>>,
        low: bool,
        run: Run,
        on_event: Bound<'py, PyAny>,
    ) -> Called<'py> {
        let repeats = self.0.file.index().has_events();
        let op = Operation::PageRank {
            params: PageRank {
                damping,
                tol,
                max_iter,
                personalize: personalize.as_deref(),
            },
            precision: if low { Precision::Low } else { Precision::Full },
            weight: if edges && repeats {
                Weight::Edges
            } else {
                Weight::Pairs
            },
        };
        self.0.call(py, &op, &run, &on_event)
    }

    /// Breadth-first search from the dense id `source` (`everygraph.Graph.bfs`).
    #[pyo3(signature = (source, direction, max_hops, run, on_event))]
    fn bfs<'py>(
        &self,
        py: Python<'py>,
        source: u32,
        direction: &str,
        max_hops: Option<u32>,
        run: Run,
        on_event: Bound<'py, PyAny>,
    ) -> Called<'py> {
        let direction = match direction {
            "in" => Direction::In,
            "both" => Direction::Both,
            _ => Direction::Out,
        };
        let op = Operation::Bfs {
            source,
            max_hops,
            direction,
        };
        self.0.call(py, &op, &run, &on_event)
    }

    /// Weakly or strongly connected components (`everygraph.Graph.components`).
    fn components<'py>(
        &self,
        py: Python<'py>,
        strong: bool,
        run: Run,
        on_event: Bound<'py, PyAny>,
    ) -> Called<'py> {
        self.0
            .call(py, &Operation::Components { strong }, &run, &on_event)
    }

    /// Core numbers (`everygraph.Graph.kcore`).
    fn kcore<'py>(&self, py: Python<'py>, run: Run, on_event: Bound<'py, PyAny>) -> Called<'py> {
        self.0.call(py, &Operation::KCore, &run, &on_event)
    }

    /// Degrees (`everygraph.Graph.degrees`), of every node or of the dense ids `nodes`,
    /// in that order; `None` among them gives a row of nulls.
    fn degrees<'py>(
        &self,
        py: Python<'py>,
        nodes: Option<Vec<Option<u32>>>,
        run: Run,
        on_event: Bound<'py, PyAny>,
    ) -> Called<'py> {
        let (table, info) = self.0.call(py, &Operation::Degrees, &run, &on_event)?;
        match (table, nodes) {
            (Some(table), Some(ids)) => {
                let rows: Vec<u32> = ids.into_iter().map(|id| id.unwrap_or(NULL)).collect();
                Ok((Some(table.take(&rows)?), info))
            }
            (table, _) => Ok((table, info)),
        }
    }
}
