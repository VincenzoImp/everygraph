// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`Hierarchy`]: a stored hierarchy, read part by part.

use everygraph_core::{ByteSource, Memory, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyEvents, AdjacencyIndex};
use everygraph_format::container::{Container, Kind, ResultKey, StoredResult, Value};

use super::{HierarchyError, INTERNAL, MEMBERSHIP, NODES, PARENT, QUOTIENT, VOLUME, part};
use crate::column::{Column, GRAIN};
use crate::quality::Table;

/// A hierarchy found in a graph file: see [`Hierarchy::find`].
#[derive(Debug)]
pub struct Hierarchy<'f, S> {
    file: &'f Container<S>,
    root: StoredResult<'f, S>,
    levels: u32,
    nodes: u32,
    /// The communities of each level, level 1 first.
    counts: Vec<u32>,
}

/// The quotient graph of a level: an adjacency over the level's communities, a loop
/// at each community that has an internal pair. An entry's events are its weight: the
/// pairs between two communities, or inside one on the loop.
#[derive(Debug)]
pub struct StoredQuotient<'f> {
    index: AdjacencyIndex,
    data: &'f [u8],
    events: Option<AdjacencyEvents<'f>>,
}

impl StoredQuotient<'_> {
    /// Its lists.
    ///
    /// # Errors
    ///
    /// When the data part does not belong to the index.
    pub fn adjacency(&self) -> Result<Adjacency<'_>, HierarchyError> {
        Ok(Adjacency::new(&self.index, self.data)?)
    }

    /// The weights of its entries, checked whole; `None` when every entry weighs 1.
    ///
    /// # Errors
    ///
    /// When the section is damaged.
    pub fn events(&self) -> Result<Option<&AdjacencyEvents<'_>>, HierarchyError> {
        if let Some(e) = &self.events {
            e.check()?;
        }
        Ok(self.events.as_ref())
    }

    /// The index.
    #[must_use]
    pub fn index(&self) -> &AdjacencyIndex {
        &self.index
    }
}

impl<'f, S: ByteSource> Hierarchy<'f, S> {
    /// The hierarchy stored under `key` ([`key`](super::key)); `None` when there is none
    /// for this graph. Reads the record and the length of each part.
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

    /// The hierarchy whose root is `root`.
    pub(crate) fn of(
        file: &'f Container<S>,
        root: StoredResult<'f, S>,
    ) -> Result<Self, HierarchyError> {
        let mut head = [0_u64; 2];
        if root.len() != 2 {
            return Err(invalid("its root does not hold the levels and the nodes"));
        }
        root.read(0, &mut head)?;
        let levels = u32::try_from(head[0]).map_err(|_| invalid("too many levels"))?;
        let nodes = u32::try_from(head[1]).map_err(|_| invalid("too many nodes"))?;
        if levels == 0 || u64::from(levels) > u64::from(nodes).max(1) {
            return Err(invalid("a number of levels that cannot be"));
        }
        let mut h = Self {
            file,
            root,
            levels,
            nodes,
            counts: Vec::new(),
        };
        for level in 1..=levels {
            let count = h.typed::<u32>(&part(level, NODES))?.len();
            h.counts
                .push(u32::try_from(count).map_err(|_| invalid("too many communities"))?);
        }
        for level in 1..=levels {
            let count = u64::from(h.counts[level as usize - 1]);
            for name in [INTERNAL, VOLUME] {
                if h.typed::<u64>(&part(level, name))?.len() != count {
                    return Err(invalid("a table column of another length than the level"));
                }
            }
            if level > 1 && h.typed::<u32>(&part(level, PARENT))?.len() != count {
                return Err(invalid("parents of another length than the level"));
            }
        }
        if h.typed::<u32>(MEMBERSHIP)?.len() != u64::from(nodes) {
            return Err(invalid("a membership of another length than the graph"));
        }
        Ok(h)
    }

    /// Part `name`, holding values of type `T`.
    fn typed<T: Value>(&self, name: &str) -> Result<StoredResult<'f, S>, HierarchyError> {
        let p = self
            .file
            .result_part(&self.root, name)?
            .ok_or_else(|| HierarchyError::Invalid(format!("the part {name} is missing")))?;
        if p.value_type() != T::TYPE {
            return Err(HierarchyError::Invalid(format!(
                "the part {name} holds values of another type"
            )));
        }
        Ok(p)
    }

    /// Part `name`, read whole into a column.
    fn column<T: Value + Default + Send>(
        &self,
        memory: &Memory,
        name: &str,
    ) -> Result<Column<T>, HierarchyError> {
        let p = self.typed::<T>(name)?;
        let len = usize::try_from(p.len()).map_err(|_| invalid("a part too large"))?;
        let mut out = Column::new(memory, "hierarchy part", len, |_| T::default())?;
        p.read(0, &mut out)?;
        Ok(out)
    }

    /// The number of levels; level 1 is the coarsest, the partition.
    #[must_use]
    pub fn levels(&self) -> u32 {
        self.levels
    }

    /// The number of nodes of the graph.
    #[must_use]
    pub fn nodes(&self) -> u32 {
        self.nodes
    }

    /// The number of communities at `level`.
    ///
    /// # Errors
    ///
    /// [`HierarchyError::Argument`] for a level that does not exist.
    pub fn count(&self, level: u32) -> Result<u32, HierarchyError> {
        level
            .checked_sub(1)
            .and_then(|l| self.counts.get(l as usize))
            .copied()
            .ok_or(HierarchyError::Argument("no such level"))
    }

    /// The community table of `level`, one row per community in id order: input nodes,
    /// internal weight and volume ([`Table::cut`] gives the weight to the rest). On an
    /// unweighted graph the weights are counts of pairs, a kept loop inside.
    ///
    /// # Errors
    ///
    /// [`HierarchyError::Argument`] for a level that does not exist;
    /// [`HierarchyError::Invalid`] for a row whose internal weight is above half its
    /// volume; the file's errors and the budget's.
    pub fn table(&self, level: u32, memory: &Memory) -> Result<Table, HierarchyError> {
        self.count(level)?;
        let table = Table {
            nodes: self.column(memory, &part(level, NODES))?,
            internal: self.column(memory, &part(level, INTERNAL))?,
            volume: self.column(memory, &part(level, VOLUME))?,
        };
        let cut = table.internal.iter().zip(&table.volume[..]);
        if cut
            .into_iter()
            .any(|(&l, &s)| l.checked_mul(2).is_none_or(|l| l > s))
        {
            return Err(invalid("an internal weight above half its volume"));
        }
        Ok(table)
    }

    /// The community at `level − 1` of each community at `level`; `None` at level 1.
    ///
    /// # Errors
    ///
    /// [`HierarchyError::Invalid`] for a parent beyond the count of the level above; as
    /// [`table`](Self::table).
    pub fn parents(
        &self,
        level: u32,
        memory: &Memory,
    ) -> Result<Option<Column<u32>>, HierarchyError> {
        self.count(level)?;
        if level == 1 {
            return Ok(None);
        }
        self.parents_of(level, memory).map(Some)
    }

    /// The parents of `level`, from 2 up.
    fn parents_of(&self, level: u32, memory: &Memory) -> Result<Column<u32>, HierarchyError> {
        let parents = self.column(memory, &part(level, PARENT))?;
        below(&parents, self.counts[level as usize - 2], "a parent")?;
        Ok(parents)
    }

    /// The community at `level` of every node.
    ///
    /// # Errors
    ///
    /// As [`parents`](Self::parents).
    pub fn membership(&self, level: u32, memory: &Memory) -> Result<Column<u32>, HierarchyError> {
        self.count(level)?;
        let mut m = self.column::<u32>(memory, MEMBERSHIP)?;
        below(&m, self.counts[self.levels as usize - 1], "a membership")?;
        for at in (level + 1..=self.levels).rev() {
            let parents = self.parents_of(at, memory)?;
            par::for_each_chunk_mut(&mut m, GRAIN, |_, chunk| {
                for c in chunk {
                    *c = parents[*c as usize];
                }
            });
        }
        Ok(m)
    }

    /// The communities that hold community `community` of `level`, at levels `level − 1`
    /// down to 1. Reads one value per level.
    ///
    /// # Errors
    ///
    /// [`HierarchyError::Argument`] for a level or a community that does not exist;
    /// [`HierarchyError::Invalid`] for a parent beyond its level's count.
    pub fn ancestors(&self, level: u32, community: u32) -> Result<Vec<u32>, HierarchyError> {
        if community >= self.count(level)? {
            return Err(HierarchyError::Argument("no such community"));
        }
        let mut out = Vec::new();
        let mut c = community;
        for at in (2..=level).rev() {
            let mut p = [0_u32];
            self.typed::<u32>(&part(at, PARENT))?
                .read(u64::from(c), &mut p)?;
            if p[0] >= self.counts[at as usize - 2] {
                return Err(invalid("a parent beyond the level above"));
            }
            c = p[0];
            out.push(c);
        }
        Ok(out)
    }

    /// The communities at the finer level `at` inside community `community` of `level`,
    /// in id order.
    ///
    /// # Errors
    ///
    /// [`HierarchyError::Argument`] for levels or a community that do not exist, or
    /// `at` not below `level`; as [`parents`](Self::parents).
    pub fn descendants(
        &self,
        level: u32,
        community: u32,
        at: u32,
        memory: &Memory,
    ) -> Result<Vec<u32>, HierarchyError> {
        if community >= self.count(level)? || at <= level {
            return Err(HierarchyError::Argument("no such community or finer level"));
        }
        let count = self.count(at)?;
        let mut up = Column::new(memory, "ancestors", count as usize, |c| {
            u32::try_from(c).unwrap_or(u32::MAX)
        })?;
        for l in (level + 1..=at).rev() {
            let parents = self.parents_of(l, memory)?;
            par::for_each_chunk_mut(&mut up, GRAIN, |_, chunk| {
                for c in chunk {
                    *c = parents[*c as usize];
                }
            });
        }
        Ok((0..count)
            .filter(|&c| up[c as usize] == community)
            .collect())
    }

    /// The nodes of community `community` of `level`, increasing: the node set of
    /// `subgraph(community)`.
    ///
    /// # Errors
    ///
    /// As [`ancestors`](Self::ancestors) and [`membership`](Self::membership).
    pub fn members(
        &self,
        level: u32,
        community: u32,
        memory: &Memory,
    ) -> Result<Column<u32>, HierarchyError> {
        if community >= self.count(level)? {
            return Err(HierarchyError::Argument("no such community"));
        }
        let mut size = [0_u32];
        self.typed::<u32>(&part(level, NODES))?
            .read(u64::from(community), &mut size)?;
        let m = self.membership(level, memory)?;
        let mut out = Column::new(memory, "members", size[0] as usize, |_| 0_u32)?;
        let mut at = 0;
        for (v, _) in m.iter().enumerate().filter(|&(_, &c)| c == community) {
            *out.get_mut(at)
                .ok_or_else(|| invalid("a community holds more nodes than its count"))? =
                u32::try_from(v).unwrap_or(u32::MAX);
            at += 1;
        }
        if at != out.len() {
            return Err(invalid("a community holds fewer nodes than its count"));
        }
        Ok(out)
    }

    /// The quotient graph of `level`, read from a file held in memory (a memory map).
    ///
    /// # Errors
    ///
    /// [`HierarchyError::Invalid`] when the file is not in memory or the parts break
    /// the adjacency format or the level; as [`count`](Self::count).
    pub fn quotient(&self, level: u32) -> Result<StoredQuotient<'f>, HierarchyError> {
        let count = self.count(level)?;
        let bytes = |suffix: &str, kind: Kind| -> Result<Option<&'f [u8]>, HierarchyError> {
            let name = part(level, &format!("{QUOTIENT}.{suffix}"));
            let Some(e) = self.file.result_part_entry(&self.root, &name) else {
                return Ok(None);
            };
            if e.kind != kind {
                return Err(HierarchyError::Invalid(format!(
                    "{name} is of kind {}",
                    e.kind.code()
                )));
            }
            self.file
                .source()
                .slice(e.offset..e.offset + e.len)
                .map(Some)
                .ok_or_else(|| invalid("the file is not in memory"))
        };
        let missing = || invalid("a quotient graph is missing");
        let index = AdjacencyIndex::parse(
            bytes("index", Kind::RESULT_ADJACENCY_INDEX)?.ok_or_else(missing)?,
        )?;
        let data = bytes("data", Kind::RESULT_ADJACENCY_DATA)?.ok_or_else(missing)?;
        let events = match (
            bytes("events", Kind::RESULT_ADJACENCY_EVENTS)?,
            index.has_events(),
        ) {
            (Some(b), _) => Some(AdjacencyEvents::parse(b, &index)?),
            (None, false) => None,
            (None, true) => return Err(missing()),
        };
        if index.nodes() != count {
            return Err(invalid("a quotient graph of another size than its level"));
        }
        Adjacency::new(&index, data)?;
        Ok(StoredQuotient {
            index,
            data,
            events,
        })
    }
}

fn invalid(what: &str) -> HierarchyError {
    HierarchyError::Invalid(what.to_owned())
}

/// Checks that every value of `values` is below `count`.
fn below(values: &[u32], count: u32, what: &str) -> Result<(), HierarchyError> {
    if values.iter().any(|&c| c >= count) {
        return Err(HierarchyError::Invalid(format!(
            "{what} beyond the communities of its level"
        )));
    }
    Ok(())
}
