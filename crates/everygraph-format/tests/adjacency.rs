// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The adjacency codec against the oracle: every graph of the CI corpus (sparse-id
//! families included) and generated CSRs with arbitrary `u32` targets round-trip
//! through every access path and every decoder; on corrupted bytes every decoder
//! returns exactly what the scalar decoder returns, and nothing panics.

#![forbid(unsafe_code)]

use everygraph_core::NodeId;
use everygraph_format::adjacency::{
    Adjacency, AdjacencyIndex, Batch, DecodeError, Decoder, Encoded, encode,
};
use everygraph_oracle::corpus::{Tier, specs};
use everygraph_oracle::generators::generate;
use everygraph_oracle::invariants;
use everygraph_oracle::projection::Projection;
use everygraph_oracle::{Kernel, Output, ix, to_usize};
use proptest::prelude::*;

/// Miri interprets the tests about 1000 times slower: it runs a sample.
const MIRI: bool = cfg!(miri);

/// Few cases under Miri, and no failure file (Miri's isolation hides the file system).
fn config() -> ProptestConfig {
    ProptestConfig {
        cases: if MIRI { 2 } else { 256 },
        failure_persistence: if MIRI {
            None
        } else {
            ProptestConfig::default().failure_persistence
        },
        ..ProptestConfig::default()
    }
}

fn id(v: usize) -> NodeId {
    NodeId::try_from(v).expect("test graphs have fewer than 2^32 nodes")
}

fn list<'a>(offsets: &[u64], targets: &'a [u32], v: usize) -> &'a [u32] {
    &targets[to_usize(offsets[v])..to_usize(offsets[v + 1])]
}

/// Encodes the CSR and checks every access path of every decoder against it.
fn round_trip(offsets: &[u64], targets: &[u32]) -> Encoded {
    let encoded = encode(offsets, targets).expect("valid CSR");
    let index = AdjacencyIndex::parse(&encoded.index).expect("index written by encode");
    let n = offsets.len() - 1;
    assert_eq!(ix(index.nodes()), n);
    assert_eq!(index.entries(), targets.len() as u64);
    for (v, &o) in offsets.iter().enumerate() {
        assert_eq!(index.entry_offset(id(v)).get(), o);
    }
    for decoder in Decoder::available() {
        let adj = Adjacency::new(&index, &encoded.data)
            .unwrap()
            .with_decoder(decoder);
        let name = decoder.name();
        let mut buf = Vec::new();
        for v in 0..n {
            let node = id(v);
            let want = list(offsets, targets, v);
            assert_eq!(ix(index.degree(node)), want.len());
            assert_eq!(
                adj.neighbors(node, &mut buf).unwrap(),
                want,
                "{name} node {v}"
            );
            let ks: &[usize] = if MIRI {
                &[1, 5]
            } else {
                &[0, 1, 2, 3, 4, 5, 7, 8, 9]
            };
            let ends = [want.len().saturating_sub(1), want.len() + 1];
            for &k in ks.iter().chain(&ends) {
                let got = adj.neighbors_prefix(node, k, &mut buf).unwrap();
                assert_eq!(
                    got,
                    &want[..k.min(want.len())],
                    "{name} node {v} prefix {k}"
                );
            }
        }
        // Batches: every block, the whole range, and ranges cut anywhere.
        let mut batch = Batch::default();
        let mut ranges: Vec<_> = (0..index.blocks()).map(|b| index.block_nodes(b)).collect();
        let n32 = id(n).get();
        ranges.push(0..n32);
        if !MIRI {
            ranges.extend((0..n32).step_by(7).map(|a| a..(a + 3).min(n32)));
        }
        for r in ranges {
            // Whole ranges at once, and in batches of a few entries (at least one list
            // each, never more than the budget unless a single list is longer).
            let budgets: &[usize] = if MIRI {
                &[5]
            } else {
                &[usize::MAX, 0, 1, 5, 64]
            };
            for &budget in budgets {
                let mut at = r.start;
                let mut seen = Vec::new();
                while at < r.end {
                    let next = adj.decode_nodes(at..r.end, budget, &mut batch).unwrap();
                    assert_eq!(batch.first(), at);
                    assert_eq!(ix(next - at), batch.len(), "{name} batch {r:?}");
                    assert!(batch.len() == 1 || batch.targets().len() <= budget);
                    for (v, l) in batch.iter() {
                        assert_eq!(l, list(offsets, targets, v.index()), "{name} batch {r:?}");
                        seen.push(v.get());
                    }
                    assert!(next > at, "{name} no progress at {at}");
                    at = next;
                }
                assert_eq!(
                    seen,
                    r.clone().collect::<Vec<_>>(),
                    "{name} {r:?} budget {budget}"
                );
            }
            adj.decode_nodes(r.clone(), usize::MAX, &mut batch).unwrap();
            let all = &targets[to_usize(offsets[ix(r.start)])..to_usize(offsets[ix(r.end)])];
            assert_eq!(batch.targets(), all);
        }
        let (mut o, mut t) = (vec![u64::MAX; n + 1], vec![0u32; targets.len()]);
        adj.decode_all(&mut o, &mut t).unwrap();
        assert_eq!((&o[..], &t[..]), (offsets, targets), "{name} decode_all");
    }
    encoded
}

/// The tests above go through `Decoder::available()`; this makes sure a run with a SIMD
/// feature compiled in (the Miri SSSE3 run, AArch64, wasm `simd128`) really tests it.
#[test]
fn a_simd_decoder_compiled_in_is_available() {
    let names: Vec<_> = Decoder::available().iter().map(|d| d.name()).collect();
    assert_eq!(names[0], "scalar");
    if cfg!(all(
        any(target_arch = "x86", target_arch = "x86_64"),
        target_feature = "ssse3"
    )) {
        assert!(names.contains(&"ssse3"), "{names:?}");
    }
    if cfg!(target_arch = "aarch64") {
        assert!(names.contains(&"neon"), "{names:?}");
    }
    if cfg!(all(target_arch = "wasm32", target_feature = "simd128")) {
        assert!(names.contains(&"simd128"), "{names:?}");
    }
    assert_eq!(Decoder::best().name(), *names.last().unwrap());
}

/// The analytic adjacency: the projection's lists with each kept self-loop listed once
/// (`docs/spec/graph-semantics.md`), which is what the oracle's adjacency invariants expect.
fn with_kept_loops(
    p: &Projection,
    adj: &everygraph_oracle::projection::Adjacency,
) -> (Vec<u64>, Vec<u32>) {
    let mut offsets = vec![0u64];
    let mut targets = Vec::new();
    for v in 0..adj.n() {
        let mut l = adj.list(v).to_vec();
        if p.kept_loops(v) > 0 {
            let at = l.partition_point(|&u| ix(u) < v);
            l.insert(at, id(v).get());
        }
        targets.extend(l);
        offsets.push(targets.len() as u64);
    }
    (offsets, targets)
}

#[test]
fn the_oracle_corpus_round_trips_on_every_path() {
    let all = specs(Tier::Ci);
    // Under Miri a sample: one graph in 40, none above 300 nodes.
    let step = if MIRI { 40 } else { 1 };
    let mut checked = 0;
    for spec in all.iter().step_by(step) {
        let g = generate(spec);
        if MIRI && g.n > 300 {
            continue;
        }
        let p = Projection::new(&g);
        let ((out_offsets, out_targets), (in_offsets, in_targets)) =
            (with_kept_loops(&p, &p.out), with_kept_loops(&p, &p.inn));
        round_trip(&in_offsets, &in_targets);
        let encoded = round_trip(&out_offsets, &out_targets);
        // The decoded out-adjacency passes the oracle's adjacency invariants.
        let index = AdjacencyIndex::parse(&encoded.index).unwrap();
        let view = Adjacency::new(&index, &encoded.data).unwrap();
        let mut offsets = vec![0; out_offsets.len()];
        let mut targets = vec![0; out_targets.len()];
        view.decode_all(&mut offsets, &mut targets).unwrap();
        let out = Output::Csr { offsets, targets };
        let v = invariants::check(&Kernel::Adjacency, &p, None, &out);
        assert!(v.is_empty(), "{}: {v:?}", spec.name);
        checked += 1;
    }
    assert!(checked >= if MIRI { 3 } else { 280 }, "{checked} graphs");
}

/// Lists whose first neighbour and gaps sit on every boundary of the codes: one, two,
/// three and four bytes (2^8, 2^16, 2^24), the LEB128 boundaries the oracle's sparse-id
/// family aims at (2^7, 2^14, 2^21, 2^28), and the wrap of the 32-bit difference.
#[test]
fn ids_and_gaps_on_every_code_boundary() {
    let mut lists: Vec<Vec<u32>> = Vec::new();
    for k in [7u32, 8, 14, 16, 21, 24, 28, 31] {
        for gap in [(1u32 << k) - 1, 1 << k, (1 << k) + 1] {
            lists.push(vec![0, gap]);
            lists.push(vec![3, 3 + gap, u32::MAX]);
            lists.push(vec![u32::MAX - gap - 1, u32::MAX - 1]);
        }
    }
    lists.push(vec![0, u32::MAX]);
    lists.push(vec![u32::MAX]);
    lists.push((0..32).map(|i| i << 27).collect());
    lists.push(vec![]);
    let mut offsets = vec![0u64];
    let mut targets = Vec::new();
    for l in &lists {
        targets.extend_from_slice(l);
        offsets.push(targets.len() as u64);
    }
    round_trip(&offsets, &targets);
}

/// Id-width agnostic: shifting every target by a constant leaves every record but the
/// first word of each the same, so the size changes by at most 4 bytes per list.
#[test]
fn record_size_does_not_depend_on_where_the_ids_are() {
    let lists: Vec<Vec<u32>> = (0..if MIRI { 12u32 } else { 64 })
        .map(|v| (0..v).map(|i| 3 * i + v).collect())
        .collect();
    let size = |shift: u32| {
        let mut offsets = vec![0u64];
        let mut targets = Vec::new();
        for l in &lists {
            targets.extend(l.iter().map(|x| x + shift));
            offsets.push(targets.len() as u64);
        }
        round_trip(&offsets, &targets).data.len()
    };
    let base = size(0);
    for shift in [1 << 20, 1 << 30, 3 << 30] {
        let s = size(shift);
        assert!(
            s >= base && s <= base + 4 * lists.len(),
            "shift {shift}: {s} vs {base}"
        );
    }
}

fn arb_target() -> impl Strategy<Value = u32> {
    prop_oneof![
        0u32..64,
        any::<u32>(),
        (u32::MAX - 64)..=u32::MAX,
        (2u32..32).prop_flat_map(|k| ((1u32 << k) - 2)..=(1u32 << k) + 2),
    ]
}

fn arb_csr() -> impl Strategy<Value = (Vec<u64>, Vec<u32>)> {
    let (lists, longest) = if MIRI { (6, 12) } else { (24, 70) };
    proptest::collection::vec(
        proptest::collection::btree_set(arb_target(), 0..longest),
        0..lists,
    )
    .prop_map(|lists| {
        let mut offsets = vec![0u64];
        let mut targets = Vec::new();
        for l in lists {
            targets.extend(l);
            offsets.push(targets.len() as u64);
        }
        (offsets, targets)
    })
}

/// What every read of an adjacency returns, errors included.
fn outcomes(adj: Adjacency<'_>) -> Vec<Result<Vec<u32>, DecodeError>> {
    let index = adj.index();
    let mut out = Vec::new();
    let mut buf = Vec::new();
    for v in 0..index.nodes() {
        let id = NodeId::new(v);
        out.push(adj.neighbors(id, &mut buf).map(<[u32]>::to_vec));
        for k in [1, 3, 4, 6] {
            out.push(adj.neighbors_prefix(id, k, &mut buf).map(<[u32]>::to_vec));
        }
    }
    let mut batch = Batch::default();
    for b in 0..index.blocks() {
        out.push(
            adj.decode_nodes(index.block_nodes(b), usize::MAX, &mut batch)
                .map(|_| batch.targets().to_vec()),
        );
    }
    let mut offsets = vec![0; ix(index.nodes()) + 1];
    let mut targets = vec![0; to_usize(index.entries())];
    out.push(adj.decode_all(&mut offsets, &mut targets).map(|()| targets));
    out
}

fn decoders_agree(index_bytes: &[u8], data: &[u8]) {
    let Ok(index) = AdjacencyIndex::parse(index_bytes) else {
        return;
    };
    let Ok(adj) = Adjacency::new(&index, data) else {
        return;
    };
    let reference = outcomes(adj.with_decoder(Decoder::SCALAR));
    for decoder in Decoder::available() {
        assert_eq!(
            outcomes(adj.with_decoder(decoder)),
            reference,
            "{}",
            decoder.name()
        );
        apis_agree(adj.with_decoder(decoder));
    }
}

/// The APIs agree on the same bytes: a batch or the whole graph decodes only if every
/// list in it decodes on its own, and then to the same lists.
fn apis_agree(adj: Adjacency<'_>) {
    let index = adj.index();
    let mut buf = Vec::new();
    let lists: Vec<_> = (0..index.nodes())
        .map(|v| adj.neighbors(NodeId::new(v), &mut buf).map(<[u32]>::to_vec))
        .collect();
    let mut batch = Batch::default();
    for b in 0..index.blocks() {
        let nodes = index.block_nodes(b);
        let all_ok = lists[ix(nodes.start)..ix(nodes.end)]
            .iter()
            .all(Result::is_ok);
        let got = adj.decode_nodes(nodes.clone(), usize::MAX, &mut batch);
        assert_eq!(got.is_ok(), all_ok, "block {b}");
        if let Err(e) = got {
            assert!(
                lists[ix(e.first)].is_err(),
                "the batch error names a node whose list fails"
            );
        }
        for (v, l) in batch.iter() {
            assert_eq!(Ok(l.to_vec()), lists[v.index()], "block {b}");
        }
    }
    let mut offsets = vec![0; ix(index.nodes()) + 1];
    let mut targets = vec![0; to_usize(index.entries())];
    let all = adj.decode_all(&mut offsets, &mut targets);
    assert_eq!(all.is_ok(), lists.iter().all(Result::is_ok));
    if all.is_ok() {
        let concatenated: Vec<u32> = lists.into_iter().flat_map(Result::unwrap).collect();
        assert_eq!(targets, concatenated);
    }
}

/// Review of #30, M1: with records checked only where a batch ends, one damaged byte
/// made `decode_nodes` and `decode_all` return `[128, 0]` while `neighbors(0)` failed.
/// Every API must report the damage at node 0.
#[test]
fn one_damaged_byte_is_an_error_at_its_node_in_every_api() {
    let mut e = encode(&[0, 1, 2], &[0, 129]).unwrap();
    assert_eq!(e.data, [0x00, 0x00, 0x01, 0x00, 0x01]);
    e.data[0] = 0x01;
    let index = AdjacencyIndex::parse(&e.index).unwrap();
    let at_node_0 = DecodeError { first: 0, end: 1 };
    for decoder in Decoder::available() {
        let adj = Adjacency::new(&index, &e.data)
            .unwrap()
            .with_decoder(decoder);
        let name = decoder.name();
        let mut buf = Vec::new();
        assert_eq!(
            adj.neighbors(NodeId::new(0), &mut buf),
            Err(at_node_0),
            "{name}"
        );
        assert_eq!(
            adj.neighbors(NodeId::new(1), &mut buf),
            Ok(&[129][..]),
            "{name}"
        );
        let mut batch = Batch::default();
        assert_eq!(
            adj.decode_nodes(0..2, usize::MAX, &mut batch),
            Err(at_node_0),
            "{name}"
        );
        assert!(batch.is_empty(), "{name}");
        assert_eq!(
            adj.decode_nodes(1..2, usize::MAX, &mut batch),
            Ok(2),
            "{name}"
        );
        assert_eq!(batch.targets(), [129], "{name}");
        let (mut offsets, mut targets) = (vec![0; 3], vec![0; 2]);
        assert_eq!(
            adj.decode_all(&mut offsets, &mut targets),
            Err(at_node_0),
            "{name}"
        );
        apis_agree(adj);
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn arbitrary_csrs_round_trip((offsets, targets) in arb_csr()) {
        round_trip(&offsets, &targets);
    }

    #[test]
    fn decoders_agree_on_corrupted_records(
        (offsets, targets) in arb_csr(),
        hits in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..6),
    ) {
        let mut e = encode(&offsets, &targets).unwrap();
        if !e.data.is_empty() {
            for (at, byte) in hits {
                let i = at.index(e.data.len());
                e.data[i] = byte;
            }
        }
        decoders_agree(&e.index, &e.data);
    }

    #[test]
    fn corrupted_or_truncated_indexes_are_refused_or_read_safely(
        (offsets, targets) in arb_csr(),
        at in any::<prop::sample::Index>(),
        byte in any::<u8>(),
        cut in any::<prop::sample::Index>(),
    ) {
        let e = encode(&offsets, &targets).unwrap();
        let mut index = e.index.clone();
        let i = at.index(index.len());
        index[i] = byte;
        decoders_agree(&index, &e.data);
        let cut = cut.index(e.index.len());
        prop_assert!(AdjacencyIndex::parse(&e.index[..cut]).is_err());
        prop_assert!(AdjacencyIndex::parse(&e.index).and_then(|ix| Adjacency::new(&ix, &e.data[..e.data.len().saturating_sub(1)]).map(|_| ())).is_err() || e.data.is_empty());
    }
}
