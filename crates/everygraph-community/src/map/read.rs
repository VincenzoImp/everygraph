// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`StoredMap`]: a stored map tree, read part by part.

use everygraph_core::{ByteSource, Memory};
use everygraph_format::container::{Container, ResultKey, StoredResult};

use super::Kind;
use super::store::{INTERNAL, KIND, MEMBERSHIP, NODES, PARENT, SIBLINGS, VOLUME, decode, invalid};
use crate::column::Column;
use crate::hierarchy::read::{adjacency, column, typed};
use crate::hierarchy::{HierarchyError, StoredQuotient};
use crate::quality::Table;

/// A map tree found in a graph file: see [`StoredMap::find`]. Region 0 is the root;
/// regions are numbered breadth first, the children of each region together, by
/// decreasing node count, then smallest node.
#[derive(Debug)]
pub struct StoredMap<'f, S> {
    file: &'f Container<S>,
    root: StoredResult<'f, S>,
    regions: u32,
    nodes: u32,
    built: bool,
}

impl<'f, S: ByteSource> StoredMap<'f, S> {
    /// The map stored under `key` ([`key`](super::key)); `None` when there is none for this graph.
    ///
    /// # Errors
    ///
    /// [`HierarchyError::Invalid`] for parts missing or of the wrong length; the
    /// file's errors.
    pub fn find(file: &'f Container<S>, key: &ResultKey) -> Result<Option<Self>, HierarchyError> {
        match file.find_result(key)? {
            Some(root) => Self::of(file, root).map(Some),
            None => Ok(None),
        }
    }

    pub(crate) fn of(
        file: &'f Container<S>,
        root: StoredResult<'f, S>,
    ) -> Result<Self, HierarchyError> {
        if root.len() != 3 {
            return Err(invalid(
                "its root does not hold the regions, the nodes and the super level",
            ));
        }
        let mut head = [0_u64; 3];
        root.read(0, &mut head)?;
        let regions = u32::try_from(head[0]).map_err(|_| invalid("too many regions"))?;
        let nodes = u32::try_from(head[1]).map_err(|_| invalid("too many nodes"))?;
        if regions == 0 || head[2] > 1 {
            return Err(invalid("a root that cannot be"));
        }
        let m = Self {
            file,
            root,
            regions,
            nodes,
            built: head[2] == 1,
        };
        let lengths = [
            (
                PARENT,
                typed::<u32, S>(file, &m.root, PARENT)?.len(),
                regions,
            ),
            (KIND, typed::<u32, S>(file, &m.root, KIND)?.len(), regions),
            (NODES, typed::<u32, S>(file, &m.root, NODES)?.len(), regions),
            (
                INTERNAL,
                typed::<u64, S>(file, &m.root, INTERNAL)?.len(),
                regions,
            ),
            (
                VOLUME,
                typed::<u64, S>(file, &m.root, VOLUME)?.len(),
                regions,
            ),
            (
                MEMBERSHIP,
                typed::<u32, S>(file, &m.root, MEMBERSHIP)?.len(),
                nodes,
            ),
        ];
        if let Some((name, _, _)) = lengths
            .iter()
            .find(|(_, len, want)| *len != u64::from(*want))
        {
            return Err(invalid(&format!("the part {name} is of another length")));
        }
        Ok(m)
    }

    /// The number of regions, the root included.
    #[must_use]
    pub fn regions(&self) -> u32 {
        self.regions
    }

    /// The number of nodes of the graph.
    #[must_use]
    pub fn nodes(&self) -> u32 {
        self.nodes
    }

    /// Whether the map has a super level: the decision a rebuild keeps within its band.
    #[must_use]
    pub fn super_level(&self) -> bool {
        self.built
    }

    /// The parent of every region; the root is its own.
    ///
    /// # Errors
    ///
    /// [`HierarchyError::Invalid`] for a parent that is not a region; the file's errors
    /// and the budget's.
    pub fn parents(&self, memory: &Memory) -> Result<Column<u32>, HierarchyError> {
        let parents: Column<u32> = column(self.file, &self.root, memory, PARENT)?;
        if parents.iter().any(|&p| p >= self.regions) {
            return Err(invalid("a parent that is not a region"));
        }
        Ok(parents)
    }

    /// The kind of every region.
    ///
    /// # Errors
    ///
    /// [`HierarchyError::Invalid`] for a code that is not a kind; as
    /// [`parents`](Self::parents).
    pub fn kinds(&self, memory: &Memory) -> Result<Column<Kind>, HierarchyError> {
        let codes: Column<u32> = column(self.file, &self.root, memory, KIND)?;
        if codes.iter().any(|&c| decode(c).is_none()) {
            return Err(invalid("a kind that is not one"));
        }
        Ok(Column::new(memory, "map kinds", codes.len(), |x| {
            decode(codes[x]).unwrap_or(Kind::Root)
        })?)
    }

    /// One row per region: nodes, internal weight and volume, as
    /// [`Hierarchy::table`](crate::hierarchy::Hierarchy::table).
    ///
    /// # Errors
    ///
    /// As [`parents`](Self::parents).
    pub fn table(&self, memory: &Memory) -> Result<Table, HierarchyError> {
        Ok(Table {
            nodes: column(self.file, &self.root, memory, NODES)?,
            internal: column(self.file, &self.root, memory, INTERNAL)?,
            volume: column(self.file, &self.root, memory, VOLUME)?,
        })
    }

    /// The leaf holding every node.
    ///
    /// # Errors
    ///
    /// [`HierarchyError::Invalid`] for a value that is not a region; as
    /// [`parents`](Self::parents).
    pub fn membership(&self, memory: &Memory) -> Result<Column<u32>, HierarchyError> {
        let m: Column<u32> = column(self.file, &self.root, memory, MEMBERSHIP)?;
        if m.iter().any(|&x| x >= self.regions) {
            return Err(invalid("a membership that is not a region"));
        }
        Ok(m)
    }

    /// The sibling graph: an entry between two children of one region, weighted by the
    /// pairs of nodes between them, read from a file held in memory.
    ///
    /// # Errors
    ///
    /// [`HierarchyError::Invalid`] when the file is not in memory or the parts break
    /// the adjacency format.
    pub fn siblings(&self) -> Result<StoredQuotient<'f>, HierarchyError> {
        adjacency(self.file, &self.root, SIBLINGS, self.regions)
    }
}
