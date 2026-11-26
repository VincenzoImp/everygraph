// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A graph file as the build writes it, opened for reading: the record of its build
//! (direction, the treatment of loops and repeats, the counts of
//! `docs/spec/graph-semantics.md` § 2.8), its out-adjacency and its identifier
//! dictionary. The caller gives the container, on whatever source it chose (a range map
//! of the committed part, `docs/spec/eg-container.md` § 11).

use everygraph_core::{ByteSource, Memory, MemoryAccountant, MemoryKind, Reservation};
use everygraph_format::adjacency::AdjacencyIndex;
use everygraph_format::container::{Container, ContainerError, Provenance, Section};

use crate::build::{BuildError, OUT_DATA, OUT_INDEX, PROVENANCE};
use crate::ids::{Dictionary, DictionaryError};

/// A graph file, open.
#[derive(Debug)]
pub struct Graph<S> {
    file: Container<S>,
    record: Provenance,
    index: AdjacencyIndex,
    directed: bool,
    _index: Reservation,
}

impl<S: ByteSource> Graph<S> {
    /// Reads the record of the build and the out-adjacency's index, which is held in
    /// memory under `memory`.
    ///
    /// # Errors
    ///
    /// [`BuildError::Container`] with [`ContainerError::NoSuchSection`] for a container
    /// that holds no graph; [`BuildError::Format`] for an invalid index;
    /// [`BuildError::Budget`] when the index does not fit `memory`; read errors.
    pub fn open(file: Container<S>, memory: &Memory) -> Result<Self, BuildError> {
        let record = file.read_provenance(PROVENANCE)?;
        let directed = record
            .parameters
            .get("directed")
            .is_some_and(|d| d == "true");
        let section = file.section(OUT_INDEX)?;
        let len = usize::try_from(section.len()).map_err(|_| BuildError::TooLarge(OUT_INDEX))?;
        // The bytes, then the parsed index, which holds about as much again.
        let held = memory.reserve("the adjacency index", MemoryKind::Anonymous, widen(len) * 2)?;
        let mut bytes = vec![0_u8; len];
        section.read_at(0, &mut bytes)?;
        let index = AdjacencyIndex::parse(&bytes)?;
        let data = file
            .get(OUT_DATA)
            .ok_or_else(|| ContainerError::NoSuchSection(OUT_DATA.into()))?;
        if data.len != index.data_bytes() {
            return Err(everygraph_format::adjacency::FormatError::Length {
                expected: index.data_bytes(),
                actual: data.len,
            }
            .into());
        }
        Ok(Self {
            file,
            record,
            index,
            directed,
            _index: held,
        })
    }

    /// The container.
    #[must_use]
    pub fn file(&self) -> &Container<S> {
        &self.file
    }

    /// The record of the build: its parameters and counts, its inputs, its times.
    #[must_use]
    pub fn record(&self) -> &Provenance {
        &self.record
    }

    /// Whether the edges have a direction.
    #[must_use]
    pub fn directed(&self) -> bool {
        self.directed
    }

    /// Nodes.
    #[must_use]
    pub fn nodes(&self) -> u32 {
        self.index.nodes()
    }

    /// Edges: the events stored, as the build counted them; the out-adjacency's entries
    /// when its record does not say.
    #[must_use]
    pub fn edges(&self) -> u64 {
        self.count("counts.edges")
            .unwrap_or_else(|| self.index.entries())
    }

    /// The count `key` of the record (`counts.pairs`, `dropped.self_loops`, …).
    #[must_use]
    pub fn count(&self, key: &str) -> Option<u64> {
        self.record.parameters.get(key).and_then(|v| v.parse().ok())
    }

    /// The out-adjacency's index.
    #[must_use]
    pub fn index(&self) -> &AdjacencyIndex {
        &self.index
    }

    /// The out-adjacency's data part.
    ///
    /// # Errors
    ///
    /// The section is missing.
    pub fn data(&self) -> Result<Section<'_, S>, ContainerError> {
        self.file.section(OUT_DATA)
    }

    /// The identifier dictionary, its hash function held under `accountant`.
    ///
    /// # Errors
    ///
    /// As [`Dictionary::open`].
    pub fn dictionary<'a>(
        &'a self,
        accountant: &'a dyn MemoryAccountant,
    ) -> Result<Dictionary<'a, Section<'a, S>>, DictionaryError> {
        Dictionary::open(accountant, |name| self.file.section(name).ok())
    }
}

fn widen(x: usize) -> u64 {
    u64::try_from(x).unwrap_or(u64::MAX)
}
