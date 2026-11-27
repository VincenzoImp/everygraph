// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Point queries on a graph file that `everygraph-ingest` built: identifiers resolved
//! through its dictionary, repeated pairs counted from its `.events` sections, and
//! identifiers of any bytes — the query input a user types — resolved without error.

#![forbid(unsafe_code)]
#![allow(clippy::disallowed_methods, reason = "test code")]

use std::num::NonZeroUsize;

use everygraph_core::{Memory, MemorySource, NodeId};
use everygraph_format::container::Provenance;
use everygraph_ingest::build::{self, BuildOptions, Duplicates, GraphOptions, SelfLoops, Silent};
use everygraph_ingest::graph::Graph as File;
use everygraph_ingest::ids::{Key, MemoryScratch, OwnedKey};
use everygraph_ingest::text::{Header, TextOptions, TextReader};
use everygraph_kernels::edge_map::Compressed;
use everygraph_query::ids::resolve;
use everygraph_query::{Columns, Direction, Graph};
use proptest::prelude::*;

/// The bytes of the directed graph of the edge list `text`, loops kept, every row an
/// event.
fn built(text: &str) -> Vec<u8> {
    let text_options = TextOptions {
        header: Header::Absent,
        ..TextOptions::default()
    };
    let reader = TextReader::open(MemorySource::new(text.as_bytes()), &text_options)
        .expect("a text edge list");
    let graph = GraphOptions {
        directed: true,
        self_loops: SelfLoops::Keep,
        duplicates: Duplicates::Events,
    };
    let options = BuildOptions::new(graph, NonZeroUsize::MIN);
    let sink = MemorySource::new(Vec::new());
    let memory = Memory::unlimited();
    let unfinished =
        build::build(&reader, &options, &memory, &MemoryScratch, sink, &Silent).expect("a build");
    let (sink, _) = unfinished.commit(Provenance::default()).expect("a commit");
    sink.into_inner()
}

const EDGES: &str = "alice bob\nalice bob\nbob carol\ncarol alice\ncarol carol\ncarol carol\n";

#[test]
fn queries_read_a_built_file() {
    let bytes = built(EDGES);
    let memory = Memory::unlimited();
    let file = File::open(MemorySource::new(&bytes[..]), &memory).unwrap();
    let names = ["alice", "bob", "nobody", "carol"];
    let ids = resolve(
        &file.dictionary(&memory).unwrap(),
        names.iter().map(|&n| Key::from(n)),
        &memory,
    )
    .unwrap();
    let ids: Vec<Option<u32>> = ids.iter().map(|i| i.map(NodeId::get)).collect();
    let [Some(alice), Some(bob), None, Some(carol)] = ids[..] else {
        panic!("{ids:?}");
    };
    let (data, in_data) = (file.data().unwrap(), file.in_data().unwrap());
    let out = Compressed::mapped(file.index(), &data).unwrap();
    let inn = Compressed::mapped(file.in_index().unwrap(), &in_data).unwrap();
    let (events, _held) = file.events(&memory).unwrap().unwrap();
    let (in_events, _in_held) = file.in_events(&memory).unwrap().unwrap();
    let columns = |e| Columns {
        events: Some(e),
        ..Columns::default()
    };
    let g = Graph::directed(&out, Some(&inn))
        .unwrap()
        .with_columns(columns(&events), columns(&in_events))
        .unwrap();
    let mut r = g.reader(&memory).unwrap();
    let rows = |e: everygraph_query::EdgeRows| -> Vec<(u32, u64)> {
        (0..e.len()).map(|i| (e.target[i], e.edges[i])).collect()
    };
    let out_of_alice = r.neighbors(&[alice], Direction::Out, 10).unwrap();
    assert_eq!(rows(out_of_alice), [(bob, 2)]);
    // Carol: its loop once (two events), Alice out, Bob in.
    let mut want = vec![(alice, 1), (bob, 1), (carol, 2)];
    want.sort_unstable();
    assert_eq!(
        rows(r.neighbors(&[carol], Direction::Both, 10).unwrap()),
        want
    );
    assert!(r.has_edge(carol, carol).unwrap() && !r.has_edge(bob, bob).unwrap());
    let degrees = r.degrees(&[carol]).unwrap();
    assert_eq!(degrees.degree[0], 4);
    let common = r.common_neighbors(alice, bob, 10).unwrap();
    assert_eq!(common.neighbor.iter().copied().collect::<Vec<_>>(), [carol]);
    assert_eq!((common.edges_a[0], common.edges_b[0]), (1, 1));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any bytes given as an identifier resolve to nothing, or to a node whose key
    /// resolves to it again; every node's own key resolves to it. The input's keys are
    /// of every kind the dictionary detects: hexadecimal, integers, text.
    #[test]
    fn any_identifier_resolves_or_is_unknown(
        keys in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..12), 1..40),
        known in proptest::collection::vec("[a-fA-F0-9x]{1,6}|[a-z]{1,4}", 2..12),
    ) {
        let text: String = known.windows(2).map(|w| format!("{} {}\n", w[0], w[1])).collect();
        let bytes = built(&text);
        let memory = Memory::unlimited();
        let file = File::open(MemorySource::new(&bytes[..]), &memory).unwrap();
        let dictionary = file.dictionary(&memory).unwrap();
        let text_of = |id: NodeId| match dictionary.key(id).unwrap() {
            OwnedKey::Text(t) => t.into_bytes(),
            OwnedKey::Bytes(b) => b,
            OwnedKey::Int(i) => i.to_string().into_bytes(),
            OwnedKey::UInt(u) => u.to_string().into_bytes(),
        };
        let nodes: Vec<Vec<u8>> = (0..dictionary.len()).map(|v| text_of(NodeId::new(v))).collect();
        let asked: Vec<&[u8]> = keys
            .iter()
            .map(Vec::as_slice)
            .chain(known.iter().map(String::as_bytes))
            .chain(nodes.iter().map(Vec::as_slice))
            .collect();
        let ids = resolve(&dictionary, asked.iter().map(|&k| Key::from(k)), &memory).unwrap();
        prop_assert_eq!(ids.len(), asked.len());
        for (key, id) in asked.iter().zip(ids.iter()) {
            match id {
                Some(id) => {
                    let again = dictionary.lookup(Key::from(&text_of(*id)[..])).unwrap();
                    prop_assert_eq!(again, Some(*id));
                }
                None => prop_assert!(!nodes.iter().any(|n| n == key), "{:?} is a key", key),
            }
        }
    }
}
