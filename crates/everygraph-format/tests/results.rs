// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Results kept in the graph file (`docs/spec/eg-container.md` § 6, "Results"): the
//! graph identity, and a result served only under its own key and graph. The crash
//! guarantees of storing one are tested with the container's (`tests/container.rs`).
//!
//! Not run under Miri, like `tests/container.rs`: safe code, and hashing makes it take
//! minutes there.

#![forbid(unsafe_code)]
#![cfg(not(miri))]

use everygraph_core::{ByteSource, MemorySource};
use everygraph_format::container::{
    Container, ContainerError, GraphIdentity, Kind, Provenance, ResultKey, Value, ValueType, Writer,
};
use everygraph_oracle::rng::SplitMix64;

type Mem = MemorySource<Vec<u8>>;

fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut rng = SplitMix64::new(seed);
    (0..len).map(|_| rng.next_u64().to_le_bytes()[0]).collect()
}

fn about() -> Provenance {
    Provenance {
        engine: "everygraph test".into(),
        started: Some(5),
        finished: Some(6),
        ..Provenance::default()
    }
}

/// A graph of two adjacency sections and a dictionary section, and a section of a
/// private kind.
fn graph() -> Writer<Mem> {
    let mut w = Writer::create(Mem::default()).unwrap();
    w.add_section(
        "adjacency.out.index",
        Kind::ADJACENCY_INDEX,
        &bytes(1, 300),
        None,
    )
    .unwrap();
    w.add_section(
        "adjacency.out.data",
        Kind::ADJACENCY_DATA,
        &bytes(2, 5000),
        None,
    )
    .unwrap();
    w.add_section("ids.keys", Kind::ID_DICTIONARY, &bytes(3, 40), None)
        .unwrap();
    w.add_section("x.private", Kind::PRIVATE, &bytes(4, 10), None)
        .unwrap();
    w.commit().unwrap();
    w
}

/// The identity of a graph with no sections: another graph than `graph()`'s.
fn empty_graph() -> GraphIdentity {
    Writer::create(Mem::default()).unwrap().graph_identity()
}

fn reopen(w: Writer<Mem>) -> Container<Mem> {
    Container::open(w.into_sink()).unwrap()
}

/// The values stored under `key`, or `None`.
fn find<T: Value + Default>(file: &Container<Mem>, key: &ResultKey) -> Option<Vec<T>> {
    let stored = file.find_result(key).unwrap()?;
    let mut out = vec![T::default(); usize::try_from(stored.len()).unwrap()];
    stored.read(0, &mut out).unwrap();
    Some(out)
}

fn pagerank(graph: GraphIdentity, version: u32, deterministic: bool) -> ResultKey {
    ResultKey::new(graph, "pagerank", version, deterministic)
        .unwrap()
        .with("damping", "0.85")
        .unwrap()
        .with("iterations", "20")
        .unwrap()
}

/// Spec § 6, computed here from the TOC as the spec states it, independently of the
/// crate's code: BLAKE3 over (kind, name length, name, hash) of kinds 2–5 and 7, in name
/// order.
fn identity_by_hand<S: ByteSource>(file: &Container<S>) -> [u8; 32] {
    let mut input = Vec::new();
    for e in file
        .entries()
        .iter()
        .filter(|e| [2, 3, 4, 5, 7].contains(&e.kind.code()))
    {
        input.extend_from_slice(&e.kind.code().to_le_bytes());
        input.extend_from_slice(&u32::try_from(e.name.len()).unwrap().to_le_bytes());
        input.extend_from_slice(e.name.as_bytes());
        input.extend_from_slice(&e.hash);
    }
    *blake3::hash(&input).as_bytes()
}

#[test]
fn the_identity_covers_the_graph_sections_and_nothing_else() {
    let mut w = graph();
    let id = w.graph_identity();
    // Results, provenance, private sections and a column's layout and statistics
    // (kinds 8 and 9, which follow from the column) do not change it.
    w.add_provenance("p", &about()).unwrap();
    w.add_section("x.more", Kind::PRIVATE, b"x", Some("p"))
        .unwrap();
    w.add_section("nodes/layout/age", Kind::COLUMN_LAYOUT, b"layout", None)
        .unwrap();
    w.add_section("nodes/stats/age", Kind::COLUMN_STATS, b"stats", None)
        .unwrap();
    let key = pagerank(id, 1, true);
    w.add_result(&key, &about(), &[1.0_f64]).unwrap();
    assert_eq!(w.graph_identity(), id);
    w.commit().unwrap();
    let file = reopen(w);
    assert_eq!(file.graph_identity(), id);
    assert_eq!(*id.as_bytes(), identity_by_hand(&file));
    assert_eq!(
        file.find_result(&key)
            .unwrap()
            .unwrap()
            .provenance()
            .parameters["graph"],
        id.to_string()
    );

    // Each change to a graph section — content, name, kind, one added (a dictionary
    // section or a property column, kind 7), one removed —
    // gives a new identity.
    let changes: [fn(&mut Writer<Mem>); 6] = [
        |w| {
            w.remove("ids.keys").unwrap();
            w.add_section("ids.keys", Kind::ID_DICTIONARY, &bytes(3, 41), None)
                .unwrap();
        },
        |w| {
            w.remove("ids.keys").unwrap();
            w.add_section("ids.keyz", Kind::ID_DICTIONARY, &bytes(3, 40), None)
                .unwrap();
        },
        |w| {
            w.remove("ids.keys").unwrap();
            w.add_section("ids.keys", Kind::ADJACENCY_EVENTS, &bytes(3, 40), None)
                .unwrap();
        },
        |w| {
            w.add_section("ids.more", Kind::ID_DICTIONARY, &[], None)
                .unwrap();
        },
        |w| {
            w.remove("ids.keys").unwrap();
        },
        |w| {
            w.add_section("nodes/arrow/age", Kind::COLUMN, b"column", None)
                .unwrap();
        },
    ];
    let mut seen = vec![id];
    for change in changes {
        let mut w = graph();
        change(&mut w);
        w.commit().unwrap();
        let file = reopen(w);
        let changed = file.graph_identity();
        assert_eq!(*changed.as_bytes(), identity_by_hand(&file));
        assert!(!seen.contains(&changed));
        seen.push(changed);
    }
}

#[test]
fn a_result_is_served_only_under_its_own_key_and_graph() {
    let mut w = graph();
    let id = w.graph_identity();
    let ranks = [0.25_f64, 0.5, 0.125, 0.125];
    let fast = [0.3_f64, 0.4, 0.2, 0.1];
    w.add_result(&pagerank(id, 1, true), &about(), &ranks)
        .unwrap();
    w.add_result(&pagerank(id, 1, false), &about(), &fast)
        .unwrap();
    w.commit().unwrap();
    let file = reopen(w);

    assert_eq!(
        find::<f64>(&file, &pagerank(id, 1, true)),
        Some(ranks.to_vec())
    );
    // A `fast` result is never served for a deterministic request, nor the reverse.
    assert_eq!(
        find::<f64>(&file, &pagerank(id, 1, false)),
        Some(fast.to_vec())
    );
    // Any other key misses: a new version of the algorithm, another parameter value,
    // one parameter more or less, another operation, another graph.
    let misses = [
        pagerank(id, 2, true),
        ResultKey::new(id, "pagerank", 1, true)
            .unwrap()
            .with("damping", "0.9")
            .unwrap()
            .with("iterations", "20")
            .unwrap(),
        pagerank(id, 1, true).with("seed", "1").unwrap(),
        ResultKey::new(id, "pagerank", 1, true)
            .unwrap()
            .with("damping", "0.85")
            .unwrap(),
        ResultKey::new(id, "katz", 1, true)
            .unwrap()
            .with("damping", "0.85")
            .unwrap()
            .with("iterations", "20")
            .unwrap(),
        pagerank(empty_graph(), 1, true),
    ];
    for key in &misses {
        assert!(file.find_result(key).unwrap().is_none(), "{key:?}");
    }
    let stored = file.find_result(&pagerank(id, 1, true)).unwrap().unwrap();
    assert_eq!(stored.value_type(), ValueType::F64);
    assert_eq!(stored.provenance().engine, "everygraph test");
    assert_eq!(stored.provenance().operation, "pagerank");
    assert_eq!(stored.provenance().finished, Some(6));

    // After the graph changes, the stored result is not served for the new graph,
    // and the old key no longer matches the file.
    let mut w = Writer::update(file.into_source()).unwrap();
    w.add_section("ids.more", Kind::ID_DICTIONARY, b"z", None)
        .unwrap();
    w.commit().unwrap();
    let file = reopen(w);
    let new_id = file.graph_identity();
    assert_ne!(new_id, id);
    assert!(file.find_result(&pagerank(id, 1, true)).unwrap().is_none());
    assert!(
        file.find_result(&pagerank(new_id, 1, true))
            .unwrap()
            .is_none()
    );
    // The old result is still in the file, as a result of the earlier graph.
    assert_eq!(
        file.entries()
            .iter()
            .filter(|e| e.kind == Kind::RESULT)
            .count(),
        2
    );
}

#[test]
fn values_of_every_type_round_trip_and_floats_are_canonical() {
    let mut w = graph();
    let id = w.graph_identity();
    let key = |op: &str| ResultKey::new(id, op, 1, true).unwrap();
    let n = 10_000_u32;
    let labels: Vec<u32> = (0..n).map(|i| i.wrapping_mul(2_654_435_761)).collect();
    let counts: Vec<u64> = (0..u64::from(n)).map(|i| i << 40 | i).collect();
    let singles = [1.5_f32, -0.0, f32::from_bits(0xFFC0_0001), f32::INFINITY];
    let doubles = [-0.0_f64, f64::from_bits(0x7FF0_0000_0000_0001), 1e-300];
    w.add_result(&key("labels"), &about(), &labels).unwrap();
    w.add_result(&key("counts"), &about(), &counts).unwrap();
    w.add_result(&key("singles"), &about(), &singles).unwrap();
    w.add_result(&key("doubles"), &about(), &doubles).unwrap();
    w.add_result::<u32>(&key("empty"), &about(), &[]).unwrap();
    w.commit().unwrap();
    let file = reopen(w);

    assert_eq!(find::<u32>(&file, &key("labels")), Some(labels.clone()));
    assert_eq!(find::<u64>(&file, &key("counts")), Some(counts));
    let bits = |v: Vec<f32>| v.into_iter().map(f32::to_bits).collect::<Vec<_>>();
    assert_eq!(
        bits(find::<f32>(&file, &key("singles")).unwrap()),
        [0x3FC0_0000, 0, 0x7FC0_0000, 0x7F80_0000]
    );
    let bits = |v: Vec<f64>| v.into_iter().map(f64::to_bits).collect::<Vec<_>>();
    assert_eq!(
        bits(find::<f64>(&file, &key("doubles")).unwrap()),
        [0, 0x7FF8_0000_0000_0000, 1e-300_f64.to_bits()]
    );
    assert_eq!(find::<u32>(&file, &key("empty")), Some(Vec::new()));

    // A part of the values, from anywhere.
    let stored = file.find_result(&key("labels")).unwrap().unwrap();
    for (first, len) in [(0, 1), (4095, 3), (9000, 1000), (n - 1, 1), (n, 0)] {
        let mut part = vec![0_u32; len];
        stored.read(u64::from(first), &mut part).unwrap();
        assert_eq!(part, labels[first as usize..first as usize + len]);
    }
    assert!(matches!(
        stored.read(u64::from(n), &mut [0_u32; 1]),
        Err(ContainerError::InvalidResult(_))
    ));
    assert!(matches!(
        stored.read(0, &mut [0_u64; 1]),
        Err(ContainerError::InvalidResult(_))
    ));
}

#[test]
fn storing_under_the_same_key_replaces_the_result() {
    let mut w = graph();
    let key = ResultKey::new(w.graph_identity(), "degrees", 1, true).unwrap();
    w.add_result(&key, &about(), &[1_u32, 2, 3]).unwrap();
    w.commit().unwrap();
    let mut w = Writer::update(reopen(w).into_source()).unwrap();
    w.add_result(&key, &about(), &[7_u64]).unwrap();
    w.commit().unwrap();
    let file = reopen(w);
    assert_eq!(find::<u64>(&file, &key), Some(vec![7]));
    let results = file.entries().iter().filter(|e| e.kind == Kind::RESULT);
    assert_eq!(results.count(), 1);
    assert_eq!(file.entries().len(), 4 + 2);
}

#[test]
fn what_cannot_be_a_result_key_or_record_is_refused() {
    let mut w = graph();
    let id = w.graph_identity();
    for reserved in [
        "graph",
        "version",
        "deterministic",
        "memory",
        "threads",
        "temp_dir",
        "progress",
    ] {
        assert!(matches!(
            ResultKey::new(id, "pagerank", 1, true)
                .unwrap()
                .with(reserved, "x"),
            Err(ContainerError::InvalidResult(_))
        ));
    }
    for operation in ["", "page/rank"] {
        assert!(matches!(
            ResultKey::new(id, operation, 1, true),
            Err(ContainerError::InvalidResult(_))
        ));
    }
    let key = ResultKey::new(id, "pagerank", 1, true).unwrap();
    let named = Provenance {
        operation: "other".into(),
        ..about()
    };
    assert!(matches!(
        w.add_result(&key, &named, &[1_u32]),
        Err(ContainerError::InvalidResult(_))
    ));
    let elsewhere = ResultKey::new(empty_graph(), "pagerank", 1, true).unwrap();
    assert!(matches!(
        w.add_result(&elsewhere, &about(), &[1_u32]),
        Err(ContainerError::InvalidResult(_))
    ));
    assert_eq!(w.entries().len(), 4, "nothing was added");
}

#[test]
fn a_damaged_result_is_found_but_fails_its_check() {
    let mut w = graph();
    let key = ResultKey::new(w.graph_identity(), "degrees", 1, true).unwrap();
    w.add_result(&key, &about(), &[1_u32; 1000]).unwrap();
    w.commit().unwrap();
    let file = reopen(w);
    let stored = file.find_result(&key).unwrap().unwrap();
    file.verify_result(&stored).unwrap();
    let offset = file
        .entries()
        .iter()
        .find(|e| e.kind == Kind::RESULT)
        .unwrap()
        .offset;
    let mut image = file.into_source().into_inner();
    image[usize::try_from(offset).unwrap() + 100] ^= 1;
    let file = Container::open(Mem::new(image)).unwrap();
    // Found without reading the values (spec § 12), refused by the check.
    let stored = file.find_result(&key).unwrap().unwrap();
    assert!(matches!(
        file.verify_result(&stored),
        Err(ContainerError::Damaged { .. })
    ));
}

/// A section that another program wrote under `key`'s name, with `record` as its
/// provenance and `bytes` as its content; every hash is right.
fn planted(
    key: &ResultKey,
    kind: Kind,
    record: Option<&Provenance>,
    bytes: &[u8],
) -> Container<Mem> {
    let mut w = graph();
    let name = key.section_name().unwrap();
    let record_name = format!("{name}.provenance");
    if let Some(record) = record {
        w.add_provenance(&record_name, record).unwrap();
    }
    w.add_section(&name, kind, bytes, record.map(|_| record_name.as_str()))
        .unwrap();
    w.commit().unwrap();
    reopen(w)
}

/// The record a result stored under `key` holds.
fn record_of(key: &ResultKey) -> Provenance {
    Provenance {
        operation: key.operation().to_owned(),
        parameters: key.parameters().clone(),
        ..about()
    }
}

/// A result section: magic, value type, reserved, values.
fn result_bytes(code: u32, values: &[u8]) -> Vec<u8> {
    let mut out = b"EGRES\0\0\0".to_vec();
    out.extend_from_slice(&code.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(values);
    out
}

#[test]
fn a_planted_section_is_served_only_when_its_record_holds_the_key() {
    let id = graph().graph_identity();
    let wanted = pagerank(id, 1, true);
    let other = pagerank(id, 2, true);
    let values = result_bytes(4, &1.5_f64.to_le_bytes());
    // Under the wanted key's name, with the wanted key's record: served.
    let file = planted(&wanted, Kind::RESULT, Some(&record_of(&wanted)), &values);
    assert_eq!(find::<f64>(&file, &wanted), Some(vec![1.5]));
    // The same name, with a record of another key: the name alone is never trusted.
    let file = planted(&wanted, Kind::RESULT, Some(&record_of(&other)), &values);
    assert!(file.find_result(&wanted).unwrap().is_none());
    assert!(file.find_result(&other).unwrap().is_none());
    // Another kind under a result's name, or no record: not a result.
    let file = planted(&wanted, Kind::PRIVATE, Some(&record_of(&wanted)), &values);
    assert!(file.find_result(&wanted).unwrap().is_none());
    let file = planted(&wanted, Kind::RESULT, None, &values);
    assert!(file.find_result(&wanted).unwrap().is_none());
}

#[test]
fn a_result_section_that_breaks_its_layout_is_refused() {
    let id = graph().graph_identity();
    let key = pagerank(id, 1, true);
    let mut no_magic = result_bytes(4, &[0; 8]);
    no_magic[0] = b'X';
    for (what, bytes) in [
        (
            "shorter than its header",
            result_bytes(4, &[])[..15].to_vec(),
        ),
        ("no magic", no_magic),
        ("an unknown value type", result_bytes(9, &[0; 8])),
        ("value type 0", result_bytes(0, &[0; 8])),
        ("not a whole number of values", result_bytes(4, &[0; 12])),
        ("not a whole number of u32", result_bytes(1, &[0; 6])),
    ] {
        let file = planted(&key, Kind::RESULT, Some(&record_of(&key)), &bytes);
        assert!(
            matches!(
                file.find_result(&key),
                Err(ContainerError::InvalidResult(_))
            ),
            "{what}"
        );
    }
    // Reserved bytes are ignored (spec § 1).
    let mut reserved = result_bytes(1, &7_u32.to_le_bytes());
    reserved[12] = 1;
    let file = planted(&key, Kind::RESULT, Some(&record_of(&key)), &reserved);
    assert_eq!(find::<u32>(&file, &key), Some(vec![7]));
}
