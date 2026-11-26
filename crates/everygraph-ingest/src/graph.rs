// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A graph that [`build`](crate::build) wrote, opened for reading: its out-adjacency,
//! its identifier dictionary and what its provenance says (`docs/api-sketch.md` § 3.2).
//!
//! Opening reads the header, the table of contents, the small sections (each checked
//! against its hash: `everygraph_format::verify`, [`Depth::Fast`]) and the index of the
//! out-adjacency, which is held in memory under the budget. The data part and the
//! dictionary's large arrays are read on demand from the source.

use std::fmt;

use everygraph_core::{ByteSource, InsufficientBudget, Memory, MemoryKind, ReadError, Reservation};
use everygraph_format::adjacency::{AdjacencyIndex, FormatError};
use everygraph_format::container::{Container, ContainerError, Section};
use everygraph_format::verify::{Depth, verify};

use crate::build::{OUT_DATA, OUT_INDEX, PROVENANCE};
use crate::ids::{Dictionary, DictionaryError};

/// Why a graph file cannot be opened.
#[derive(Debug)]
#[non_exhaustive]
pub enum OpenError {
    /// The source failed to deliver bytes.
    Read(ReadError),
    /// The file is not a graph, or fails its quick integrity check: the section it is
    /// in (`None` for the header and the table of contents) and what is wrong.
    Corrupt {
        /// The section, if the problem is in one.
        section: Option<String>,
        /// What is wrong: one sentence, without a full stop.
        what: String,
    },
    /// The index of the out-adjacency does not fit the budget.
    Budget(InsufficientBudget),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(e) => e.fmt(f),
            Self::Corrupt {
                section: Some(s),
                what,
            } => write!(f, "{s}: {what}"),
            Self::Corrupt {
                section: None,
                what,
            } => f.write_str(what),
            Self::Budget(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for OpenError {}

fn corrupt(section: &str, what: impl fmt::Display) -> OpenError {
    OpenError::Corrupt {
        section: Some(section.to_owned()),
        what: what.to_string(),
    }
}

impl From<ContainerError> for OpenError {
    fn from(e: ContainerError) -> Self {
        match e {
            ContainerError::Read(e) => Self::Read(e),
            other => Self::Corrupt {
                section: None,
                what: other.to_string(),
            },
        }
    }
}

/// A graph file, open.
pub struct Graph<S> {
    file: Container<S>,
    index: AdjacencyIndex,
    _index_held: Reservation,
    directed: bool,
    edges: u64,
}

impl<S> fmt::Debug for Graph<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Graph")
            .field("nodes", &self.index.nodes())
            .field("edges", &self.edges)
            .field("directed", &self.directed)
            .finish_non_exhaustive()
    }
}

impl<S: ByteSource + Sync> Graph<S> {
    /// Opens the graph held by `source`, with the index under `memory`.
    ///
    /// # Errors
    ///
    /// [`OpenError::Corrupt`] for a file that is not a graph or fails the quick check;
    /// [`OpenError::Read`] when the source fails; [`OpenError::Budget`] when the index
    /// does not fit.
    pub fn open(source: S, memory: &Memory) -> Result<Self, OpenError> {
        let report = verify(&source, Depth::Fast, memory).map_err(OpenError::Read)?;
        if let Some(p) = report.problems.into_iter().next() {
            return Err(OpenError::Corrupt {
                section: p.section,
                what: p.what,
            });
        }
        let file = Container::open(source)?;
        let provenance = file
            .read_provenance(PROVENANCE)
            .map_err(|e| corrupt(PROVENANCE, e))?;
        let parameter = |key: &str| {
            provenance
                .parameters
                .get(key)
                .ok_or_else(|| corrupt(PROVENANCE, format!("no parameter {key}")))
        };
        let directed = parameter("directed")?
            .parse()
            .map_err(|_| corrupt(PROVENANCE, "directed is not true or false"))?;
        let edges = parameter("counts.edges")?
            .parse()
            .map_err(|_| corrupt(PROVENANCE, "counts.edges is not a count"))?;
        let section = file.section(OUT_INDEX)?;
        let len = usize::try_from(section.len()).map_err(|_| corrupt(OUT_INDEX, "too large"))?;
        let mut held = memory
            .reserve("adjacency index", MemoryKind::Anonymous, section.len())
            .map_err(OpenError::Budget)?;
        let mut bytes = vec![0_u8; len];
        section.read_at(0, &mut bytes).map_err(OpenError::Read)?;
        let index =
            AdjacencyIndex::parse(&bytes).map_err(|e: FormatError| corrupt(OUT_INDEX, e))?;
        drop(bytes);
        let heap = u64::try_from(index.heap_bytes()).unwrap_or(u64::MAX);
        held.try_resize(heap).map_err(OpenError::Budget)?;
        if file.get(OUT_DATA).map(|e| e.len) != Some(index.data_bytes()) {
            return Err(corrupt(
                OUT_DATA,
                "its length is not the one its index gives",
            ));
        }
        Ok(Self {
            file,
            index,
            _index_held: held,
            directed,
            edges,
        })
    }

    /// Nodes.
    #[must_use]
    pub fn nodes(&self) -> u32 {
        self.index.nodes()
    }

    /// Stored edges: every event counts (`docs/api-sketch.md` § 3.2, `num_edges`).
    #[must_use]
    pub fn edges(&self) -> u64 {
        self.edges
    }

    /// Whether the graph is directed.
    #[must_use]
    pub fn directed(&self) -> bool {
        self.directed
    }

    /// The index of the out-adjacency: the lists of an undirected graph.
    #[must_use]
    pub fn index(&self) -> &AdjacencyIndex {
        &self.index
    }

    /// The data part of the out-adjacency.
    ///
    /// # Errors
    ///
    /// None for a file that [`open`](Self::open) accepted; the container's error
    /// otherwise.
    pub fn data(&self) -> Result<Section<'_, S>, ContainerError> {
        self.file.section(OUT_DATA)
    }

    /// The identifier dictionary, whose resident part is held under `memory`.
    ///
    /// # Errors
    ///
    /// [`DictionaryError::Corrupt`] for a dictionary that does not match the adjacency;
    /// budget and read errors.
    pub fn dictionary<'a>(
        &'a self,
        memory: &'a Memory,
    ) -> Result<Dictionary<'a, Section<'a, S>>, DictionaryError> {
        let d = Dictionary::open(memory.accountant(), |name| self.file.section(name).ok())?;
        if d.len() != self.nodes() {
            return Err(DictionaryError::Corrupt(
                "the dictionary and the adjacency differ in nodes",
            ));
        }
        Ok(d)
    }

    /// The container.
    #[must_use]
    pub fn container(&self) -> &Container<S> {
        &self.file
    }
}
