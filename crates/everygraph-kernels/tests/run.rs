// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operations by name (`run`): a result is stored under the graph it was computed on,
//! and served only under its own operation and with one value per node.

#![forbid(unsafe_code)]

use everygraph_core::{Memory, MemorySource};
use everygraph_format::container::{Container, ContainerError, Kind, Provenance, Writer};
use everygraph_kernels::edge_map::{Csr, Plan, Settings};
use everygraph_kernels::pagerank::PageRank;
use everygraph_kernels::run::{
    Column, Direction, Lanes, Operation, Output, Precision, Weight, run, store, stored,
};

fn values(output: &Output) -> &[u32] {
    match &output.columns[0].1 {
        Column::U32 { values, .. } => values.as_slice().unwrap(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_result_of_one_graph_is_not_stored_in_another() {
    let (offsets, targets) = ([0, 1, 2], [1, 0]);
    let csr = Csr::new(&offsets, &targets).unwrap();
    let memory = Memory::unlimited();
    let output = Plan::with(&csr, &memory, Settings::default(), |plan| {
        run(&Operation::Degrees, &Lanes::out(&plan, false), &memory)
    })
    .unwrap();
    let mut computed_on = Writer::create(MemorySource::new(Vec::new())).unwrap();
    computed_on
        .add_section("adjacency.out.index", Kind::ADJACENCY_INDEX, b"a", None)
        .unwrap();
    let graph = computed_on.graph_identity();
    // The file was replaced while the run computed: it now holds another graph.
    let mut replaced = Writer::create(MemorySource::new(Vec::new())).unwrap();
    replaced
        .add_section("adjacency.out.index", Kind::ADJACENCY_INDEX, b"b", None)
        .unwrap();
    let refused = store(
        &mut replaced,
        &Operation::Degrees,
        graph,
        &Provenance::default(),
        &output,
    );
    assert!(
        matches!(refused, Err(ContainerError::InvalidResult(_))),
        "{refused:?}"
    );
}

#[test]
fn a_stored_result_is_found_under_its_own_operation_only() {
    // The triangle 0 1 2 and the edge 2 3, as symmetric lists.
    let (offsets, targets) = ([0, 2, 4, 7, 8], [1, 2, 0, 2, 0, 1, 3, 2]);
    let csr = Csr::new(&offsets, &targets).unwrap();
    let memory = Memory::unlimited();
    let output = Plan::with(&csr, &memory, Settings::default(), |plan| {
        run(&Operation::Degrees, &Lanes::out(&plan, false), &memory)
    })
    .unwrap();
    assert_eq!(values(&output), [2, 2, 3, 1]);

    let mut writer = Writer::create(MemorySource::new(Vec::new())).unwrap();
    let graph = writer.graph_identity();
    store(
        &mut writer,
        &Operation::Degrees,
        graph,
        &Provenance::default(),
        &output,
    )
    .unwrap();
    writer.commit().unwrap();
    let file = Container::open(writer.into_sink()).unwrap();
    // A result is served only with one value per node.
    assert!(
        stored(&file, &Operation::Degrees, false, 5, &memory)
            .unwrap()
            .is_none()
    );
    let found = stored(&file, &Operation::Degrees, false, 4, &memory)
        .unwrap()
        .unwrap();
    assert_eq!(found.columns[0].0, "degree");
    assert_eq!(values(&found), [2, 2, 3, 1]);
    for other in [
        Operation::KCore,
        Operation::Bfs {
            source: 0,
            max_hops: None,
            direction: Direction::Out,
        },
    ] {
        assert!(stored(&file, &other, false, 4, &memory).unwrap().is_none());
    }
    let damped = |damping, weight| Operation::PageRank {
        params: PageRank {
            damping,
            ..PageRank::default()
        },
        precision: Precision::Full,
        weight,
    };
    let key = |op: Operation<'_>| op.key(file.graph_identity()).unwrap();
    assert_ne!(
        key(damped(0.85, Weight::Pairs)),
        key(damped(0.5, Weight::Pairs))
    );
    // Counted edges give other ranks; pairs keep the key every earlier version wrote.
    assert_ne!(
        key(damped(0.85, Weight::Edges)),
        key(damped(0.85, Weight::Pairs))
    );
    assert!(
        !damped(0.85, Weight::Pairs)
            .parameters()
            .iter()
            .any(|(name, _)| *name == "weight")
    );
    assert_ne!(
        key(Operation::Components { strong: true }),
        key(Operation::Components { strong: false })
    );
    // One table per seed set, and three columns of a directed graph: never kept.
    let seeds = [1_u32];
    let personalized = Operation::PageRank {
        params: PageRank {
            personalize: Some(&seeds),
            ..PageRank::default()
        },
        precision: Precision::Full,
        weight: Weight::Pairs,
    };
    assert!(!personalized.stored(false) && damped(0.85, Weight::Pairs).stored(true));
    assert!(Operation::Degrees.stored(false) && !Operation::Degrees.stored(true));
}
