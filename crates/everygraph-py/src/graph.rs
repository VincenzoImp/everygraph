// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A graph for Python (`docs/api-sketch.md` § 3.2): a `.eg` file mapped by its stable
//! range, or the bytes of a graph built in memory, with the memory budget and threads
//! it was obtained with. The kernels run on its out-adjacency with the interpreter
//! released; their results stay with the budget until Python lets go of them.

use std::fs::File;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::error::{self, Fail, Failure};
use crate::kernels::{self, Call, Extra, Output};
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
        Ok(Self {
            file,
            budget,
            memory,
            path,
            ids,
        })
    }

    /// The graph file.
    pub(crate) fn file(&self) -> &GraphFile<Bytes> {
        &self.file
    }

    /// The budget the graph was opened with, for what its tables derive (`sizes`).
    pub(crate) fn memory(&self) -> &Memory {
        &self.memory
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

impl Graph {
    /// Runs the kernel `f` of `operation` as one phase, on `threads` threads with a
    /// budget of `memory` bytes for its state; returns its table and what its `meta`
    /// gains, with the budget's peak.
    fn call<'py>(
        &self,
        py: Python<'py>,
        operation: &'static str,
        memory: u64,
        threads: NonZeroUsize,
        on_event: &Bound<'py, PyAny>,
        f: impl FnOnce(&Call<'_>, &Shared) -> Result<Output, Failure> + Send,
    ) -> PyResult<(Table, Bound<'py, PyDict>)> {
        let budget = budget(memory);
        let out = run(py, threads, &budget, on_event, |events| {
            let memory = Memory::from(Arc::clone(&budget));
            let call = Call {
                operation,
                budget: &budget,
                memory: &memory,
            };
            events.send(Event::Begin(operation, None));
            let out = f(&call, &self.0);
            if out.is_ok() {
                events.send(Event::End(operation, 0));
            }
            out
        })?
        .map_err(|e| e.into_err(py))?;
        let mut table = Table::per_node(Arc::clone(&self.0), out.columns);
        for (name, ids) in out.ids {
            table = table.with_ids(name, ids);
        }
        let info = PyDict::new(py);
        for (key, value) in out.extra {
            match value {
                Extra::Count(c) => info.set_item(key, c)?,
                Extra::Real(r) => info.set_item(key, r)?,
                Extra::Flag(b) => info.set_item(key, b)?,
            }
        }
        info.set_item("peak_memory_bytes", budget.peak())?;
        Ok((table, info))
    }
}

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

    /// `PageRank` (`everygraph.Graph.pagerank`); `personalize` holds dense ids.
    #[pyo3(signature = (damping, tol, max_iter, weight, personalize, low, memory, threads, on_event))]
    #[expect(clippy::too_many_arguments, reason = "the keywords of Graph.pagerank")]
    fn pagerank<'py>(
        &self,
        py: Python<'py>,
        damping: f64,
        tol: f64,
        max_iter: u32,
        weight: bool,
        personalize: Option<Vec<u32>>,
        low: bool,
        memory: u64,
        threads: NonZeroUsize,
        on_event: Bound<'py, PyAny>,
    ) -> PyResult<(Table, Bound<'py, PyDict>)> {
        self.call(py, "pagerank", memory, threads, &on_event, |call, g| {
            let p = PageRank {
                damping,
                tol,
                max_iter,
                personalize: personalize.as_deref(),
            };
            kernels::pagerank(call, g, &p, weight, low)
        })
    }

    /// Breadth-first search from the dense id `source` (`everygraph.Graph.bfs`).
    #[pyo3(signature = (source, direction, max_hops, memory, threads, on_event))]
    #[expect(clippy::too_many_arguments, reason = "the keywords of Graph.bfs")]
    fn bfs<'py>(
        &self,
        py: Python<'py>,
        source: u32,
        direction: &str,
        max_hops: Option<u32>,
        memory: u64,
        threads: NonZeroUsize,
        on_event: Bound<'py, PyAny>,
    ) -> PyResult<(Table, Bound<'py, PyDict>)> {
        self.call(py, "bfs", memory, threads, &on_event, |call, g| {
            kernels::bfs(call, g, source, direction, max_hops)
        })
    }

    /// Weakly or strongly connected components (`everygraph.Graph.components`).
    fn components<'py>(
        &self,
        py: Python<'py>,
        strong: bool,
        memory: u64,
        threads: NonZeroUsize,
        on_event: Bound<'py, PyAny>,
    ) -> PyResult<(Table, Bound<'py, PyDict>)> {
        self.call(py, "components", memory, threads, &on_event, |call, g| {
            kernels::components(call, g, strong)
        })
    }

    /// Core numbers (`everygraph.Graph.kcore`).
    fn kcore<'py>(
        &self,
        py: Python<'py>,
        memory: u64,
        threads: NonZeroUsize,
        on_event: Bound<'py, PyAny>,
    ) -> PyResult<(Table, Bound<'py, PyDict>)> {
        self.call(py, "kcore", memory, threads, &on_event, kernels::kcore)
    }

    /// Degrees (`everygraph.Graph.degrees`), of every node or of the dense ids `nodes`,
    /// in that order; `None` among them gives a row of nulls.
    fn degrees<'py>(
        &self,
        py: Python<'py>,
        nodes: Option<Vec<Option<u32>>>,
        memory: u64,
        threads: NonZeroUsize,
        on_event: Bound<'py, PyAny>,
    ) -> PyResult<(Table, Bound<'py, PyDict>)> {
        let (table, info) =
            self.call(py, "degrees", memory, threads, &on_event, kernels::degrees)?;
        match nodes {
            Some(ids) => {
                let rows: Vec<u32> = ids.into_iter().map(|id| id.unwrap_or(NULL)).collect();
                Ok((table.take(&rows)?, info))
            }
            None => Ok((table, info)),
        }
    }
}
