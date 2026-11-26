// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A graph for Python (`docs/api-sketch.md` § 3.2): a `.eg` file mapped by its stable
//! range, or the bytes of a graph built in memory, with the memory budget and threads
//! it was obtained with. The kernels run on its out-adjacency with the interpreter
//! released; their results stay with the budget until Python lets go of them.

use std::fs::File;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::builder::{BinaryBuilder, FixedSizeBinaryBuilder, StringBuilder};
use arrow_array::{ArrayRef, Int8Array, Int16Array, Int32Array, Int64Array};
use arrow_array::{UInt8Array, UInt16Array, UInt32Array, UInt64Array};
use arrow_schema::DataType;
use everygraph_core::{Budget, Memory, MemorySource, NodeId};
use everygraph_format::container::Container;
use everygraph_ingest::graph::Graph as GraphFile;
use everygraph_ingest::ids::{ColumnType, Key, OwnedKey};
use everygraph_io::{FileSource, MmapSource};
use pyo3::prelude::*;

use crate::error::{self, Fail, Failure};
use crate::source::Bytes;
use crate::table::{NULL, Table};

/// The memory budget of a graph's calls: `limit` bytes, its large buffers on pages of
/// their own once the allocator policy (PR #219) is merged.
pub(crate) fn budget(limit: u64) -> Arc<Budget> {
    Arc::new(Budget::new(limit))
}

/// What a graph and its results share.
pub(crate) struct Shared {
    file: GraphFile<Bytes>,
    budget: Arc<Budget>,
    memory: Memory,
    threads: NonZeroUsize,
    path: Option<PathBuf>,
    ids: ColumnType,
}

impl Shared {
    /// Opens the graph of `bytes` under `budget`.
    pub(crate) fn open(
        bytes: Bytes,
        path: Option<PathBuf>,
        budget: Arc<Budget>,
        threads: NonZeroUsize,
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
            threads,
            path,
            ids,
        })
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
    /// [`id_type`](Self::id_type).
    pub(crate) fn keys(&self, ids: impl ExactSizeIterator<Item = u32>) -> Result<ArrayRef, String> {
        let dictionary = self
            .file
            .dictionary(&self.memory)
            .map_err(|e| e.to_string())?;
        let keys = ids.map(|id| match id {
            NULL => Ok(None),
            id => dictionary
                .key(NodeId::new(id))
                .map(Some)
                .map_err(|e| e.to_string()),
        });
        let keys: Vec<Option<OwnedKey>> = keys.collect::<Result<_, _>>()?;
        let int = |k: &OwnedKey| match *k {
            OwnedKey::Int(v) => Some(v),
            _ => None,
        };
        let uint = |k: &OwnedKey| match *k {
            OwnedKey::UInt(v) => Some(v),
            _ => None,
        };
        let narrow = "an identifier wider than its column";
        macro_rules! ints {
            ($array:ty, $get:expr) => {
                Arc::new(
                    keys.iter()
                        .map(|k| k.as_ref().and_then($get).map(TryInto::try_into).transpose())
                        .collect::<Result<$array, _>>()
                        .map_err(|_| narrow.to_owned())?,
                ) as ArrayRef
            };
        }
        Ok(match self.id_type() {
            DataType::Int8 => ints!(Int8Array, int),
            DataType::Int16 => ints!(Int16Array, int),
            DataType::Int32 => ints!(Int32Array, int),
            DataType::Int64 => Arc::new(
                keys.iter()
                    .map(|k| k.as_ref().and_then(int))
                    .collect::<Int64Array>(),
            ),
            DataType::UInt8 => ints!(UInt8Array, uint),
            DataType::UInt16 => ints!(UInt16Array, uint),
            DataType::UInt32 => ints!(UInt32Array, uint),
            DataType::UInt64 => Arc::new(
                keys.iter()
                    .map(|k| k.as_ref().and_then(uint))
                    .collect::<UInt64Array>(),
            ),
            DataType::Binary | DataType::FixedSizeBinary(_) => {
                let bytes = keys.iter().map(|k| match k {
                    Some(OwnedKey::Bytes(b)) => Some(b.as_slice()),
                    _ => None,
                });
                match self.ids {
                    ColumnType::FixedBinary { width } => {
                        let mut b = FixedSizeBinaryBuilder::new(i32::from(width));
                        for v in bytes {
                            match v {
                                Some(v) => b.append_value(v).map_err(|e| e.to_string())?,
                                None => b.append_null(),
                            }
                        }
                        Arc::new(b.finish())
                    }
                    _ => {
                        let mut b = BinaryBuilder::new();
                        bytes.for_each(|v| b.append_option(v));
                        Arc::new(b.finish())
                    }
                }
            }
            _ => {
                let mut b = StringBuilder::new();
                for k in &keys {
                    match k {
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
                let found = if let Ok(v) = key.extract::<u64>() {
                    dictionary.lookup(Key::UInt(v))
                } else if let Ok(v) = key.extract::<i64>() {
                    dictionary.lookup(Key::Int(v))
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

/// Opens the graph file at `path` with a budget of `memory` bytes and `threads` threads.
#[pyfunction]
pub(crate) fn open(
    py: Python<'_>,
    path: PathBuf,
    memory: u64,
    threads: NonZeroUsize,
) -> PyResult<Graph> {
    let opened = py.detach(|| open_path(&path, budget(memory), threads));
    opened
        .map(|s| Graph(Arc::new(s)))
        .map_err(|e| e.into_err(py))
}

/// Opens the graph file at `path`, mapped by its stable range (`docs/spec/eg-container.md`
/// § 11), so that a commit by another writer never changes a mapped byte.
pub(crate) fn open_path(
    path: &Path,
    budget: Arc<Budget>,
    threads: NonZeroUsize,
) -> Result<Shared, Fail> {
    let os = |e| Fail::Os(e, path.display().to_string());
    let file = File::open(path).map_err(os)?;
    let source = FileSource::from_file(file.try_clone().map_err(os)?).map_err(os)?;
    let range = Container::open(source)
        .map_err(|e| error::open(path, e.into(), budget.limit()))?
        .stable_range();
    let map = MmapSource::from_file_range(&file, range).map_err(os)?;
    Shared::open(Bytes::File(map), Some(path.to_path_buf()), budget, threads)
}

/// A graph of the bytes `bytes`, built in memory.
pub(crate) fn in_memory(
    bytes: Vec<u8>,
    budget: Arc<Budget>,
    threads: NonZeroUsize,
) -> Result<Shared, Fail> {
    Shared::open(
        Bytes::Memory(MemorySource::new(bytes)),
        None,
        budget,
        threads,
    )
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

    /// The memory budget, in bytes.
    #[getter]
    fn memory(&self) -> u64 {
        self.0.budget.limit()
    }

    /// The most bytes the graph's budget has held so far.
    #[getter]
    fn peak_memory(&self) -> u64 {
        self.0.budget.peak()
    }

    /// The threads its calls use.
    #[getter]
    fn threads(&self) -> usize {
        self.0.threads.get()
    }

    /// Bytes of the graph: the file, or what is held in memory.
    #[getter]
    fn bytes(&self) -> u64 {
        use everygraph_core::ByteSource;
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
}

impl Graph {
    pub(crate) fn failure(
        &self,
        operation: &str,
        e: everygraph_kernels::edge_map::Error,
    ) -> Failure {
        error::kernel(operation, e, self.0.budget.limit())
    }
}
