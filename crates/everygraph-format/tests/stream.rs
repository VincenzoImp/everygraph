// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The list-by-list encoder writes the bytes of `encode`, and the `.events` section
//! round-trips and is refused when damaged (`docs/spec/eg-adjacency.md` § 3.1, § 5).

#![forbid(unsafe_code)]

use everygraph_core::{Budget, Memory};
use everygraph_format::adjacency::{
    AdjacencyEvents, AdjacencyIndex, EncodeError, FormatError, IndexBuilder, append_record, encode,
    encode_events, events_section_bytes, record_bound,
};
use proptest::prelude::*;

const MIRI: bool = cfg!(miri);

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

fn arb_lists() -> impl Strategy<Value = Vec<Vec<u32>>> {
    let target = prop_oneof![0u32..64, any::<u32>(), (u32::MAX - 64)..=u32::MAX];
    let (lists, longest) = if MIRI { (6, 12) } else { (40, 90) };
    proptest::collection::vec(
        proptest::collection::btree_set(target, 0..longest)
            .prop_map(|s| s.into_iter().collect::<Vec<_>>()),
        0..lists,
    )
}

/// The data and index the list-by-list encoder writes.
fn stream(lists: &[Vec<u32>], events: bool) -> (Vec<u8>, Vec<u8>) {
    let memory = Memory::unlimited();
    let nodes = u32::try_from(lists.len()).unwrap();
    let mut index = IndexBuilder::new(&memory, nodes).unwrap();
    let mut data = Vec::new();
    for (v, list) in lists.iter().enumerate() {
        let before = data.len();
        let degree = append_record(u32::try_from(v).unwrap(), list.iter().copied(), &mut data);
        let record = (data.len() - before) as u64;
        assert!(record <= record_bound(list.len() as u64));
        index.push(degree.unwrap(), record).unwrap();
    }
    (data, index.finish(events).unwrap().0)
}

fn csr(lists: &[Vec<u32>]) -> (Vec<u64>, Vec<u32>) {
    let mut offsets = vec![0u64];
    let targets: Vec<u32> = lists.iter().flatten().copied().collect();
    for l in lists {
        offsets.push(offsets.last().unwrap() + l.len() as u64);
    }
    (offsets, targets)
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn list_by_list_is_byte_identical_to_encode(lists in arb_lists()) {
        let (offsets, targets) = csr(&lists);
        let whole = encode(&offsets, &targets).unwrap();
        let (data, index) = stream(&lists, false);
        prop_assert_eq!(&data, &whole.data);
        prop_assert_eq!(&index, &whole.index);
    }

    #[test]
    fn events_round_trip(
        multiplicities in proptest::collection::vec(1u64..5, 1..300),
        extra in any::<prop::sample::Index>(),
    ) {
        let mut multiplicities = multiplicities;
        let at = extra.index(multiplicities.len());
        multiplicities[at] += 1;
        let entries = multiplicities.len() as u64;
        let events: u64 = multiplicities.iter().sum();
        let section = encode_events(entries, events, multiplicities.iter().copied()).unwrap();
        prop_assert_eq!(section.len() as u64, events_section_bytes(entries, events).unwrap());
        let index = one_list_index(entries, true);
        let parsed = AdjacencyEvents::parse(&section, &index).unwrap();
        prop_assert_eq!(parsed.events(), events);
        prop_assert_eq!(parsed.multiplicities().unwrap().collect::<Vec<_>>(), multiplicities.clone());
        let mut start = 0;
        for (e, &m) in multiplicities.iter().enumerate() {
            prop_assert_eq!(parsed.event_range(e as u64).unwrap(), start..start + m);
            start += m;
        }
    }
}

/// The index of one node listing `0..entries`, with or without flag bit 1.
fn one_list_index(entries: u64, events: bool) -> AdjacencyIndex {
    let list: Vec<u32> = (0..u32::try_from(entries).unwrap()).collect();
    let (_, index) = stream(&[list], events);
    AdjacencyIndex::parse(&index).unwrap()
}

#[test]
fn the_events_flag_is_the_only_difference_in_the_index() {
    let lists = vec![vec![1, 2], vec![], vec![0, 5, 9]];
    let (_, plain) = stream(&lists, false);
    let (_, flagged) = stream(&lists, true);
    assert_eq!(plain.len(), flagged.len());
    let differ: Vec<usize> = (0..plain.len())
        .filter(|&i| plain[i] != flagged[i])
        .collect();
    assert_eq!(differ, vec![12]);
    assert_eq!(flagged[12], 2);
    assert!(AdjacencyIndex::parse(&flagged).unwrap().has_events());
    assert!(!AdjacencyIndex::parse(&plain).unwrap().has_events());
}

#[test]
fn the_index_builder_refuses_what_encode_refuses() {
    let memory = Memory::unlimited();
    let mut index = IndexBuilder::new(&memory, 1).unwrap();
    assert!(matches!(index.push(2, 1), Err(EncodeError::Offsets(_))));
    assert!(matches!(index.push(0, 1), Err(EncodeError::Offsets(_))));
    index.push(1, 2).unwrap();
    assert!(matches!(index.push(1, 2), Err(EncodeError::Offsets(_))));
    let short = IndexBuilder::new(&memory, 2).unwrap();
    assert!(matches!(short.finish(false), Err(EncodeError::Offsets(_))));
    let mut data = Vec::new();
    assert_eq!(
        append_record(3, [4, 4], &mut data),
        Err(EncodeError::NotIncreasing { node: 3 })
    );
    assert!(data.is_empty());
}

#[test]
fn the_index_builder_reserves_its_memory() {
    let budget = std::sync::Arc::new(Budget::new(1000));
    let memory = Memory::from(std::sync::Arc::clone(&budget));
    assert!(IndexBuilder::new(&memory, 100).is_err());
    let index = IndexBuilder::new(&memory, 10).unwrap();
    assert_eq!(budget.used(), 120);
    drop(index);
    assert_eq!(budget.used(), 0);
}

#[test]
fn encode_events_refuses_bad_multiplicities() {
    let bad = |entries, events, m: &[u64]| {
        assert!(
            matches!(
                encode_events(entries, events, m.iter().copied()),
                Err(EncodeError::Offsets(_))
            ),
            "{m:?}"
        );
    };
    bad(2, 2, &[1, 1]);
    bad(2, 3, &[1, 1]);
    bad(2, 3, &[3, 0]);
    bad(2, 3, &[1, 2, 1]);
    bad(2, 4, &[1, 2]);
    bad(3, 5, &[2, 3]);
    assert!(encode_events(2, 3, [1, 2]).is_ok());
}

/// Header fields of a `.events` section.
fn with_header(section: &[u8], at: usize, value: &[u8]) -> Vec<u8> {
    let mut s = section.to_vec();
    s[at..at + value.len()].copy_from_slice(value);
    s
}

#[test]
fn hostile_events_sections_are_refused_at_open() {
    let section = encode_events(3, 5, [1, 3, 1]).unwrap();
    let index = one_list_index(3, true);
    let refused = |bytes: &[u8], index: &AdjacencyIndex| {
        AdjacencyEvents::parse(bytes, index).expect_err("must be refused")
    };
    assert!(AdjacencyEvents::parse(&section, &index).is_ok());
    assert_eq!(refused(&section[..31], &index), FormatError::Truncated);
    assert_eq!(
        refused(&with_header(&section, 0, b"X"), &index),
        FormatError::BadMagic
    );
    assert_eq!(
        refused(&with_header(&section, 8, &2u32.to_le_bytes()), &index),
        FormatError::UnsupportedVersion(2)
    );
    assert_eq!(
        refused(&with_header(&section, 12, &1u32.to_le_bytes()), &index),
        FormatError::UnsupportedFlags(1)
    );
    let invalid = |e: FormatError| assert!(matches!(e, FormatError::Invalid(_)), "{e:?}");
    invalid(refused(&section, &one_list_index(3, false)));
    invalid(refused(&section, &one_list_index(4, true)));
    invalid(refused(
        &with_header(&section, 24, &3u64.to_le_bytes()),
        &index,
    ));
    // Huge counts: the length is computed with checked arithmetic, never wraps.
    for events in [u64::MAX, u64::MAX / 3, 1 << 40] {
        let e = refused(&with_header(&section, 24, &events.to_le_bytes()), &index);
        assert!(
            matches!(e, FormatError::Length { .. } | FormatError::Invalid(_)),
            "{e:?}"
        );
    }
    let mut longer = section.clone();
    longer.extend_from_slice(&[0; 8]);
    assert!(matches!(
        refused(&longer, &index),
        FormatError::Length { .. }
    ));
}

#[test]
fn a_damaged_sequence_is_refused_at_first_use() {
    let section = encode_events(3, 5, [1, 3, 1]).unwrap();
    let index = one_list_index(3, true);
    let mut refused = 0;
    for bit in 32 * 8..section.len() * 8 {
        let mut damaged = section.clone();
        damaged[bit / 8] ^= 1 << (bit % 8);
        let parsed = AdjacencyEvents::parse(&damaged, &index).unwrap();
        match parsed.check() {
            Err(_) => {
                refused += 1;
                assert!(parsed.event_range(0).is_err());
                assert!(parsed.multiplicities().is_err());
            }
            // Another valid sequence: it still ends at `events` and increases.
            Ok(()) => {
                let m: Vec<u64> = parsed.multiplicities().unwrap().collect();
                assert_eq!(m.iter().sum::<u64>(), 5);
                assert!(m.iter().all(|&m| m >= 1));
            }
        }
    }
    assert!(refused > 0);
}

/// Lists long enough, and runs of empty lists many enough, to close blocks by entries,
/// by nodes and by a list longer than a block: the list-by-list index is still the one
/// `encode` writes.
#[test]
fn list_by_list_matches_encode_across_block_boundaries() {
    if MIRI {
        return;
    }
    let sizes: [&[usize]; 4] = [
        &[40_000, 30_000, 1, 70_000, 0, 65_536, 65_535, 2],
        &[0; 70_000],
        &[1; 140_000],
        &[65_537, 0, 0, 65_536, 1],
    ];
    for lens in sizes {
        let lists: Vec<Vec<u32>> = lens
            .iter()
            .enumerate()
            .map(|(v, &len)| {
                let step = u32::try_from(v % 3 + 1).unwrap();
                (0..u32::try_from(len).unwrap()).map(|x| x * step).collect()
            })
            .collect();
        let (offsets, targets) = csr(&lists);
        let whole = encode(&offsets, &targets).unwrap();
        let index = AdjacencyIndex::parse(&whole.index).unwrap();
        assert!(index.blocks() > 1);
        let (data, streamed) = stream(&lists, false);
        assert!(data == whole.data);
        assert!(streamed == whole.index);
    }
}
