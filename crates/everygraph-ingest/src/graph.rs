// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A graph that [`build`](crate::build) wrote, opened for reading: its out-adjacency,
//! its in-adjacency when the file holds one (a directed graph), its identifier
//! dictionary and what its provenance says (`docs/api-sketch.md` § 3.2).
//!
//! Opening reads the header, the table of contents, the small sections (each checked
//! against its hash: `everygraph_format::verify`, [`Depth::Fast`]) and the index of each
//! adjacency, which is held in memory under the budget. The data part and the
//! dictionary's large arrays are read on demand from the source.

use std::fmt;

use everygraph_core::{ByteSource, InsufficientBudget, Memory, MemoryKind, ReadError, Reservation};
use everygraph_format::adjacency::{AdjacencyEvents, AdjacencyIndex, FormatError};
use everygraph_format::container::{Container, ContainerError, Section};
use everygraph_format::verify::{Depth, verify};

use crate::build::{
    BuildError, IN_DATA, IN_EVENTS, IN_INDEX, OUT_DATA, OUT_EVENTS, OUT_INDEX, PROVENANCE,
    read_section,
};
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

/// One adjacency of a graph file: its index, held in memory, and its section names.
struct Lists {
    index: AdjacencyIndex,
    _held: Reservation,
    data: &'static str,
    events: &'static str,
}

impl Lists {
    fn open<S: ByteSource>(
        file: &Container<S>,
        [index_name, data, events]: [&'static str; 3],
        memory: &Memory,
    ) -> Result<Self, OpenError> {
        let mut held = Reservation::empty(memory, "adjacency index", MemoryKind::Anonymous);
        let bytes = read_section(file.source(), file.get(index_name), index_name, &mut held)
            .map_err(|e| match e {
                BuildError::Budget(b) => OpenError::Budget(b),
                BuildError::Container(ContainerError::Read(e)) => OpenError::Read(e),
                other => corrupt(index_name, other),
            })?;
        let index =
            AdjacencyIndex::parse(&bytes).map_err(|e: FormatError| corrupt(index_name, e))?;
        drop(bytes);
        let heap = u64::try_from(index.heap_bytes()).unwrap_or(u64::MAX);
        held.try_resize(heap).map_err(OpenError::Budget)?;
        if file.get(data).map(|e| e.len) != Some(index.data_bytes()) {
            return Err(corrupt(data, "its length is not the one its index gives"));
        }
        Ok(Self {
            index,
            _held: held,
            data,
            events,
        })
    }
}

/// A graph file, open.
pub struct Graph<S> {
    file: Container<S>,
    out: Lists,
    inn: Option<Lists>,
    directed: bool,
    edges: u64,
}

impl<S> fmt::Debug for Graph<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Graph")
            .field("nodes", &self.out.index.nodes())
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
    /// [`OpenError::Read`] when the source fails; [`OpenError::Budget`] when an index
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
        let out = Lists::open(&file, [OUT_INDEX, OUT_DATA, OUT_EVENTS], memory)?;
        let inn = match file.get(IN_INDEX) {
            Some(_) => Some(Lists::open(&file, [IN_INDEX, IN_DATA, IN_EVENTS], memory)?),
            None => None,
        };
        let agree = |i: &AdjacencyIndex, o: &AdjacencyIndex| {
            (i.nodes(), i.entries(), i.has_events()) == (o.nodes(), o.entries(), o.has_events())
        };
        if inn.as_ref().is_some_and(|i| !agree(&i.index, &out.index)) {
            return Err(corrupt(
                IN_INDEX,
                "its nodes, entries or events flag are not the out-adjacency's",
            ));
        }
        Ok(Self {
            file,
            out,
            inn,
            directed,
            edges,
        })
    }

    /// Nodes.
    #[must_use]
    pub fn nodes(&self) -> u32 {
        self.out.index.nodes()
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
        &self.out.index
    }

    /// The data part of the out-adjacency.
    ///
    /// # Errors
    ///
    /// None for a file that [`open`](Self::open) accepted; the container's error
    /// otherwise.
    pub fn data(&self) -> Result<Section<'_, S>, ContainerError> {
        self.file.section(self.out.data)
    }

    /// The number of events of each entry of the out-adjacency, from its `.events`
    /// section; `None` when no pair repeats, so that every entry has one event.
    ///
    /// # Errors
    ///
    /// [`OpenError::Corrupt`] for a section that is not the index's, or that is not
    /// held in memory (a map or a heap buffer).
    pub fn events(&self) -> Result<Option<AdjacencyEvents<'_>>, OpenError> {
        self.events_of(&self.out)
    }

    /// The index of the in-adjacency of a directed graph (DESIGN D6); `None` when the
    /// file does not hold one: an undirected graph, or one built without it.
    #[must_use]
    pub fn in_index(&self) -> Option<&AdjacencyIndex> {
        self.inn.as_ref().map(|l| &l.index)
    }

    /// The data part of the in-adjacency.
    ///
    /// # Errors
    ///
    /// [`ContainerError::NoSuchSection`] when the file holds no in-adjacency.
    pub fn in_data(&self) -> Result<Section<'_, S>, ContainerError> {
        self.file
            .section(self.inn.as_ref().map_or(IN_DATA, |l| l.data))
    }

    /// As [`events`](Self::events), for the in-adjacency; `None` as well when the file
    /// holds no in-adjacency.
    ///
    /// # Errors
    ///
    /// As [`events`](Self::events).
    pub fn in_events(&self) -> Result<Option<AdjacencyEvents<'_>>, OpenError> {
        self.inn.as_ref().map_or(Ok(None), |l| self.events_of(l))
    }

    fn events_of<'a>(&'a self, lists: &'a Lists) -> Result<Option<AdjacencyEvents<'a>>, OpenError> {
        if !lists.index.has_events() {
            return Ok(None);
        }
        let name = lists.events;
        let entry = self
            .file
            .get(name)
            .ok_or_else(|| corrupt(name, "missing, though the index announces it"))?;
        let bytes = self
            .file
            .source()
            .slice(entry.offset..entry.offset + entry.len)
            .ok_or_else(|| corrupt(name, "not held in memory"))?;
        let events = AdjacencyEvents::parse(bytes, &lists.index).map_err(|e| corrupt(name, e))?;
        Ok(Some(events))
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

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use everygraph_core::{MemorySource, NodeId};
    use everygraph_format::container::Provenance;

    use super::*;
    use crate::build::{self, BuildOptions, GraphOptions, Silent};
    use crate::ids::{MemoryScratch, OwnedKey};
    use crate::text::{TextOptions, TextReader};

    /// The bytes of the graph of the edge list `text`.
    fn built(text: &str, directed: bool) -> Vec<u8> {
        let reader = TextReader::open(MemorySource::new(text.as_bytes()), &TextOptions::default())
            .expect("a text edge list");
        let graph = GraphOptions {
            directed,
            ..GraphOptions::default()
        };
        let options = BuildOptions::new(graph, NonZeroUsize::MIN);
        let memory = Memory::unlimited();
        let sink = MemorySource::new(Vec::new());
        let unfinished = build::build(&reader, &options, &memory, &MemoryScratch, sink, &Silent)
            .expect("a build");
        unfinished
            .commit(Provenance::default())
            .expect("a commit")
            .0
            .into_inner()
    }

    #[test]
    fn opens_what_the_build_wrote() {
        let bytes = built("5 7\n7 9\n9 5\n5 7\n", false);
        let memory = Memory::unlimited();
        let g = Graph::open(MemorySource::new(&bytes[..]), &memory).expect("a graph");
        assert_eq!((g.nodes(), g.edges(), g.directed()), (3, 4, false));
        assert_eq!(g.index().entries(), 6);
        let d = g.dictionary(&memory).expect("its dictionary");
        let keys: Vec<OwnedKey> = (0..3)
            .map(|i| d.key(NodeId::new(i)).expect("a key"))
            .collect();
        assert_eq!(keys, [5, 7, 9].map(OwnedKey::UInt));
        let events = g
            .events()
            .expect("a sound section")
            .expect("a repeated pair");
        let counts: Vec<u64> = events.multiplicities().expect("counts").collect();
        assert_eq!(counts.iter().sum::<u64>(), 8);
        assert_eq!(counts.iter().filter(|&&m| m == 2).count(), 2);
        assert!(g.in_index().is_none() && g.in_events().expect("none").is_none());
        let bytes = built("1 2\n1 2\n3 1\n", true);
        let d = Graph::open(MemorySource::new(&bytes[..]), &memory).expect("a graph");
        assert!(d.directed());
        let inn = d.in_index().expect("an in-adjacency");
        assert_eq!(
            (inn.degree(NodeId::new(0)), inn.degree(NodeId::new(1))),
            (1, 1)
        );
        assert_eq!(d.in_data().expect("its data").len(), inn.data_bytes());
        let counts: Vec<u64> = d
            .in_events()
            .expect("a sound section")
            .expect("a repeated pair")
            .multiplicities()
            .expect("counts")
            .collect();
        assert_eq!(counts, [1, 2]);
    }

    #[test]
    fn no_events_without_repeated_pairs() {
        let bytes = built("1 2\n2 3\n", false);
        let g = Graph::open(MemorySource::new(&bytes[..]), &Memory::unlimited()).expect("a graph");
        assert!(g.events().expect("no section").is_none());
    }

    #[test]
    fn refuses_what_is_not_a_whole_graph() {
        let memory = Memory::unlimited();
        let not_ours = Graph::open(MemorySource::new(&b"not a graph"[..]), &memory);
        assert!(matches!(not_ours, Err(OpenError::Corrupt { .. })));
        let mut bytes = built("1 2\n2 3\n", false);
        let file = Container::open(MemorySource::new(&bytes[..])).expect("a container");
        let at = usize::try_from(file.get(OUT_INDEX).expect("an index").offset).expect("small");
        bytes[at] ^= 0xFF;
        let damaged = Graph::open(MemorySource::new(&bytes[..]), &memory);
        assert!(
            matches!(&damaged, Err(OpenError::Corrupt { section: Some(s), .. }) if s == OUT_INDEX),
            "{damaged:?}"
        );
    }

    #[test]
    fn the_index_is_held_under_the_budget() {
        let bytes = built("1 2\n2 3\n", false);
        let tiny = Memory::from(std::sync::Arc::new(everygraph_core::Budget::new(16)));
        let refused = Graph::open(MemorySource::new(&bytes[..]), &tiny);
        assert!(matches!(refused, Err(OpenError::Budget(_))), "{refused:?}");
    }
}
