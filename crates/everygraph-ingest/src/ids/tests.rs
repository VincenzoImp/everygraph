// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests of the whole dictionary against a reference computed the obvious way, and
//! against hostile sections.

use std::collections::BTreeMap;
use std::io;
use std::sync::Mutex;

use everygraph_core::{ByteSource, MemoryAccountant, MemorySource, NodeId};
use proptest::prelude::*;

use super::kind::MAX_FIXED_WIDTH;
use super::*;
use everygraph_core::Budget;

/// One identifier as the tests hold it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum In {
    Int(i64),
    UInt(u64),
    Bytes(Vec<u8>),
}

impl In {
    fn key(&self) -> Key<'_> {
        match self {
            Self::Int(v) => Key::Int(*v),
            Self::UInt(v) => Key::UInt(*v),
            Self::Bytes(b) => Key::Bytes(b),
        }
    }
}

fn text(s: impl Into<String>) -> In {
    In::Bytes(s.into().into_bytes())
}

/// Keys in chunks of `chunk`; the position of a key is its index.
struct Keys {
    column: ColumnType,
    keys: Vec<In>,
    chunk: usize,
}

impl KeySource for Keys {
    fn column(&self) -> ColumnType {
        self.column
    }

    fn chunks(&self) -> usize {
        self.keys.len().div_ceil(self.chunk)
    }

    fn read_chunk(
        &self,
        chunk: usize,
        f: &mut dyn FnMut(u64, Key<'_>) -> Result<(), DictionaryError>,
    ) -> Result<(), DictionaryError> {
        let start = chunk * self.chunk;
        let end = (start + self.chunk).min(self.keys.len());
        for (i, k) in self.keys[start..end].iter().enumerate() {
            f(u64::try_from(start + i).unwrap(), k.key())?;
        }
        Ok(())
    }
}

/// A scratch in memory that counts the writers made for each purpose.
#[derive(Default)]
struct CountingScratch {
    created: Mutex<Vec<&'static str>>,
}

impl CountingScratch {
    fn count(&self, purpose: &str) -> usize {
        self.created
            .lock()
            .unwrap()
            .iter()
            .filter(|p| **p == purpose)
            .count()
    }
}

impl Scratch for CountingScratch {
    type Source = MemorySource<Vec<u8>>;
    type Writer = Vec<u8>;

    fn create(&self, purpose: &'static str) -> io::Result<Vec<u8>> {
        self.created.lock().unwrap().push(purpose);
        Ok(Vec::new())
    }

    fn seal(&self, writer: Vec<u8>) -> io::Result<Self::Source> {
        Ok(MemorySource::new(writer))
    }
}

/// The ids the dictionary must assign, computed the obvious way (keys that do not fit
/// the kind are left out): normalized key -> id.
fn reference(
    kind: KeyKind,
    column: ColumnType,
    keys: &[In],
    order: IdOrder,
) -> BTreeMap<Vec<u8>, u32> {
    let mut first: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
    for (pos, k) in keys.iter().enumerate() {
        let mut buf = [0; MAX_FIXED_WIDTH];
        if let Some(norm) = kind.normalize(column, k.key(), &mut buf) {
            first.entry(norm.to_vec()).or_insert(pos);
        }
    }
    let mut sorted: Vec<(Vec<u8>, usize)> = first.into_iter().collect();
    if order.resolve(kind) == IdOrder::FirstAppearance {
        sorted.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
    }
    sorted
        .into_iter()
        .enumerate()
        .map(|(i, (k, _))| (k, u32::try_from(i).unwrap()))
        .collect()
}

/// Checks every key and every id of `dict` against the reference.
fn check<S: ByteSource + Sync>(
    dict: &Dictionary<'_, S>,
    column: ColumnType,
    keys: &[In],
    order: IdOrder,
) {
    let kind = dict.kind();
    let expected = reference(kind, column, keys, order);
    assert_eq!(
        u64::from(dict.len()),
        u64::try_from(expected.len()).unwrap()
    );
    for k in keys {
        let mut buf = [0; MAX_FIXED_WIDTH];
        let Some(norm) = kind.normalize(column, k.key(), &mut buf) else {
            assert_eq!(dict.lookup(k.key()).unwrap(), None);
            continue;
        };
        let id = expected[norm];
        assert_eq!(
            dict.lookup(k.key()).unwrap(),
            Some(NodeId::new(id)),
            "{k:?}"
        );
        assert_eq!(dict.id_of_member(k.key()).unwrap(), Some(NodeId::new(id)));
    }
    let mut out = Vec::new();
    for (norm, &id) in &expected {
        dict.normalized_key(NodeId::new(id), &mut out).unwrap();
        assert_eq!(&out, norm);
        assert_eq!(
            dict.key(NodeId::new(id)).unwrap(),
            kind.render(column, norm)
        );
        assert_eq!(dict.graph_position(NodeId::new(id)).unwrap(), id);
    }
    assert!(matches!(
        dict.key(NodeId::new(dict.len())),
        Err(DictionaryError::UnknownId(_))
    ));
    dict.verify().unwrap();
}

fn build_all<'a, Sc: Scratch>(
    column: ColumnType,
    keys: &[In],
    options: BuildOptions,
    budget: &'a Budget,
    scratch: &'a Sc,
) -> Built<'a, Sc::Source> {
    let source = Keys {
        column,
        keys: keys.to_vec(),
        chunk: 97,
    };
    build_dictionary(&source, &options, budget, scratch).unwrap()
}

fn build<'a, Sc: Scratch>(
    column: ColumnType,
    keys: &[In],
    options: BuildOptions,
    budget: &'a Budget,
    scratch: &'a Sc,
) -> Dictionary<'a, Sc::Source> {
    build_all(column, keys, options, budget, scratch).dictionary
}

/// Sections by name.
type Sections = BTreeMap<&'static str, Vec<u8>>;

/// Every section's bytes, by name.
fn section_bytes<S: ByteSource>(dict: &Dictionary<'_, S>) -> Vec<(&'static str, Vec<u8>)> {
    dict.sections()
        .map(|(name, s)| {
            let mut b = vec![0; usize::try_from(s.len()).unwrap()];
            s.read_at(0, &mut b).unwrap();
            (name, b)
        })
        .collect()
}

fn open_bytes<'a>(
    bytes: &Sections,
    budget: &'a Budget,
) -> Result<Dictionary<'a, MemorySource<Vec<u8>>>, DictionaryError> {
    Dictionary::open(budget, |name| {
        bytes.get(name).map(|b| MemorySource::new(b.clone()))
    })
}

fn reopen<'a>(
    dict: &Dictionary<'_, MemorySource<Vec<u8>>>,
    budget: &'a Budget,
) -> Dictionary<'a, MemorySource<Vec<u8>>> {
    open_bytes(&section_bytes(dict).into_iter().collect(), budget).unwrap()
}

fn splitmix(z: u64) -> u64 {
    everygraph_oracle::rng::SplitMix64::new(z).next_u64()
}

/// Ethereum-like addresses: `distinct` of them, drawn `len` times, some in upper case.
fn addresses(distinct: u64, len: u64) -> Vec<In> {
    (0..len)
        .map(|i| {
            let a = splitmix(i) % distinct;
            let mut s = format!(
                "0x{:016x}{:016x}{:08x}",
                splitmix(a),
                splitmix(a + 1),
                splitmix(a + 2) >> 32
            );
            if i % 3 == 0 {
                s = s.to_ascii_uppercase().replacen("0X", "0x", 1);
            }
            text(s)
        })
        .collect()
}

/// URL-like user names of varying length.
fn users(distinct: u64, len: u64) -> Vec<In> {
    let sites = [
        "https://twitter.com/",
        "https://github.com/",
        "https://t.me/",
    ];
    (0..len)
        .map(|i| {
            let a = splitmix(i ^ 77) % distinct;
            text(format!(
                "{}user{}",
                sites[usize::try_from(a % 3).unwrap()],
                a
            ))
        })
        .collect()
}

const HEX40: KeyKind = KeyKind::Hex {
    prefixed: true,
    digits: 40,
};

#[test]
fn every_kind_order_and_layout_matches_the_reference_and_survives_reopening() {
    let utf8 = ColumnType::Utf8;
    let cases: Vec<(ColumnType, Vec<In>, KeyKind)> = vec![
        (utf8, addresses(3000, 9000), HEX40),
        (utf8, users(3000, 9000), KeyKind::Bytes),
        (
            ColumnType::Int { bits: 64 },
            (0..5000)
                .map(|i| In::Int(i64::try_from(splitmix(i) % 4000).unwrap()))
                .collect(),
            KeyKind::Integers,
        ),
        (
            ColumnType::Int { bits: 64 },
            (0..5000)
                .map(|i| In::Int(i64::from_ne_bytes(splitmix(i % 1500).to_ne_bytes())))
                .collect(),
            KeyKind::Integers,
        ),
        (
            ColumnType::UInt { bits: 32 },
            (0..5000).map(|i| In::UInt(splitmix(i) % 100)).collect(),
            KeyKind::Integers,
        ),
        (
            utf8,
            (0..3000)
                .map(|i| text((splitmix(i) % 2000).to_string()))
                .collect(),
            KeyKind::Integers,
        ),
        (
            utf8,
            (0..3000)
                .map(|i| {
                    let a = splitmix(i) % 1000;
                    text(format!(
                        "{:08x}-{:04x}-{:04X}-{:04x}-{:012x}",
                        a,
                        a % 7,
                        a % 11,
                        a % 13,
                        splitmix(a) >> 16
                    ))
                })
                .collect(),
            KeyKind::Uuid,
        ),
        (
            ColumnType::FixedBinary { width: 5 },
            (0..3000)
                .map(|i| In::Bytes(splitmix(i % 800).to_le_bytes()[..5].to_vec()))
                .collect(),
            KeyKind::Fixed { width: 5 },
        ),
        (
            utf8,
            // Bitcoin-like: a version byte, 20 bytes and 4 more, in base58.
            (0..3000)
                .map(|i| {
                    let a = splitmix(i) % 900;
                    let mut b = vec![0_u8];
                    for j in 0..3 {
                        b.extend_from_slice(&splitmix(a * 3 + j).to_le_bytes());
                    }
                    b.truncate(25);
                    In::Bytes(super::kind::base58_encode(&b))
                })
                .collect(),
            KeyKind::Base58 { width: 25 },
        ),
    ];
    for (column, keys, kind) in &cases {
        for order in [
            IdOrder::Natural,
            IdOrder::FirstAppearance,
            IdOrder::SortedKey,
        ] {
            for layout in [Layout::Split, Layout::SlotOrdered] {
                let budget = Budget::new(1 << 30);
                let options = BuildOptions::default()
                    .with_order(order)
                    .with_layout(layout);
                let built = build_all(*column, keys, options, &budget, &MemoryScratch);
                let dict = built.dictionary;
                assert_eq!(built.invalid.count, 0);
                assert_eq!(dict.kind(), *kind);
                assert_eq!(dict.order(), order.resolve(*kind));
                check(&dict, *column, keys, order);
                let again = reopen(&dict, &budget);
                check(&again, *column, keys, order);
                assert_eq!(section_bytes(&again), section_bytes(&dict));
                drop((dict, again));
                assert_eq!(budget.available(), budget.limit(), "everything released");
            }
        }
    }
}

#[test]
fn hexadecimal_keys_are_found_in_any_case_and_come_back_lowercase_with_0x() {
    let keys = vec![
        text("0XAbCdEf0123456789abcdef0123456789ABCDEF01"),
        text("0x00000000000000000000000000000000000000ff"),
    ];
    let budget = Budget::new(1 << 28);
    let dict = build(
        ColumnType::Utf8,
        &keys,
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    let id = dict
        .lookup(Key::from("0xABCDEF0123456789ABCDEF0123456789ABCDEF01"))
        .unwrap()
        .unwrap();
    assert_eq!(id, NodeId::new(0));
    assert_eq!(
        dict.key(id).unwrap(),
        OwnedKey::Text("0xabcdef0123456789abcdef0123456789abcdef01".into())
    );
    assert_eq!(
        dict.lookup(Key::from("abcdef0123456789abcdef0123456789abcdef01"))
            .unwrap(),
        None
    );
    assert_eq!(
        dict.lookup(Key::from("0x10000000000000000000000000000000000000ff"))
            .unwrap(),
        None
    );
    assert_eq!(dict.lookup(Key::Int(5)).unwrap(), None);
}

/// One stray value is an invalid row; it does not change the identity of the other
/// nodes.
#[test]
fn a_stray_key_is_an_invalid_row_and_changes_no_other_node() {
    let clean = addresses(150_000, 200_000);
    let budget = Budget::new(1 << 30);
    let reference = build(
        ColumnType::Utf8,
        &clean,
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    let mut dirty = clean.clone();
    dirty.insert(5000, text("0x"));
    dirty.push(text("not-an-address"));
    dirty.push(text(""));
    let built = build_all(
        ColumnType::Utf8,
        &dirty,
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    assert_eq!(built.dictionary.kind(), HEX40);
    assert_eq!(built.dictionary.len(), reference.len());
    assert_eq!(built.invalid.count, 3);
    let positions: Vec<u64> = built.invalid.first.iter().map(|k| k.position).collect();
    assert_eq!(positions, vec![5000, 200_001, 200_002]);
    assert_eq!(built.invalid.first[0].key, "0x");
    check(
        &built.dictionary,
        ColumnType::Utf8,
        &dirty,
        IdOrder::Natural,
    );
    // `ids="string"` keeps every key as text: case matters, nothing is invalid.
    let as_text = build_all(
        ColumnType::Utf8,
        &dirty,
        BuildOptions::default().with_kind(KeyKind::Bytes),
        &budget,
        &MemoryScratch,
    );
    assert_eq!(as_text.dictionary.kind(), KeyKind::Bytes);
    assert_eq!(as_text.invalid.count, 0);
    assert!(as_text.dictionary.len() > reference.len());
    // A kind that cannot hold the column is refused.
    let source = Keys {
        column: ColumnType::Utf8,
        keys: dirty,
        chunk: 100,
    };
    let options = BuildOptions::default().with_kind(KeyKind::Fixed { width: 3 });
    assert!(matches!(
        build_dictionary(&source, &options, &budget, &MemoryScratch),
        Err(DictionaryError::Unsupported(_))
    ));
}

/// 309 unsigned keys drawn from 0..3·10^9 are not dense; under a
/// 64 MiB budget they build through the hash function, with a small resident part.
#[test]
fn sparse_integers_below_2_to_the_32_take_the_hashed_form() {
    let keys: Vec<In> = (0..309)
        .map(|i| In::UInt(splitmix(i) % 3_000_000_000))
        .collect();
    let budget = Budget::new(64 << 20);
    let dict = build(
        ColumnType::UInt { bits: 64 },
        &keys,
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    assert!(dict.sections().any(|(name, _)| name == "ids.parts"));
    assert!(dict.sections().all(|(name, _)| name != "ids.dense"));
    assert!(dict.resident_bytes() < 1 << 16, "{}", dict.resident_bytes());
    assert!(budget.peak() < 32 << 20, "{}", budget.peak());
    check(
        &dict,
        ColumnType::UInt { bits: 64 },
        &keys,
        IdOrder::Natural,
    );
}

/// Dense keys keep the bitmap; the same keys with the bitmap turned off get the same
/// ids through the hash function; a key of 2^32 or more sends the build to the hashed
/// form.
#[test]
fn dense_integers_keep_the_bitmap_and_the_same_ids_either_way() {
    let u = ColumnType::UInt { bits: 64 };
    let keys: Vec<In> = (0..50_000)
        .map(|i| In::UInt(splitmix(i) % 100_000))
        .collect();
    let budget = Budget::new(1 << 30);
    let dense = build(u, &keys, BuildOptions::default(), &budget, &MemoryScratch);
    assert!(dense.sections().any(|(name, _)| name == "ids.dense"));
    let options = BuildOptions {
        dense: false,
        ..BuildOptions::default()
    };
    let hashed = build(u, &keys, options, &budget, &MemoryScratch);
    assert!(hashed.sections().any(|(name, _)| name == "ids.parts"));
    for k in &keys {
        assert_eq!(
            dense.lookup(k.key()).unwrap(),
            hashed.lookup(k.key()).unwrap()
        );
    }
    check(&dense, u, &keys, IdOrder::Natural);
    let mut wide = keys.clone();
    wide.push(In::UInt(1 << 40));
    let built = build(u, &wide, BuildOptions::default(), &budget, &MemoryScratch);
    assert!(built.sections().any(|(name, _)| name == "ids.parts"));
    check(&built, u, &wide, IdOrder::Natural);
    // The rule itself: at most 64 bits of bitmap per key.
    assert!(build::dense_enough(10, Some(639)));
    assert!(!build::dense_enough(10, Some(640)));
    assert!(build::dense_enough(0, None));
}

/// The section bytes of a build — and so every id — are the same under any budget,
/// and a small budget really spills.
#[test]
fn ids_do_not_depend_on_the_budget_and_a_small_budget_spills() {
    let utf8 = ColumnType::Utf8;
    for (keys, order, layout) in [
        (
            addresses(160_000, 200_000),
            IdOrder::FirstAppearance,
            Layout::Split,
        ),
        (
            addresses(160_000, 200_000),
            IdOrder::SortedKey,
            Layout::SlotOrdered,
        ),
        (users(160_000, 200_000), IdOrder::SortedKey, Layout::Split),
        (
            users(160_000, 200_000),
            IdOrder::FirstAppearance,
            Layout::SlotOrdered,
        ),
    ] {
        let options = BuildOptions::default()
            .with_order(order)
            .with_layout(layout);
        let big = Budget::new(1 << 32);
        let big_scratch = CountingScratch::default();
        let reference_dict = build(utf8, &keys, options, &big, &big_scratch);
        check(&reference_dict, utf8, &keys, order);
        let reference = section_bytes(&reference_dict);
        let big_runs = big_scratch.count("id dictionary run");
        let budget = 12 << 20;
        let small = Budget::new(budget);
        let scratch = CountingScratch::default();
        let dict = build(utf8, &keys, options, &small, &scratch);
        // Byte-identical sections: every id, key and slot is the same.
        assert_eq!(section_bytes(&dict), reference, "{order:?} {layout:?}");
        assert!(small.peak() <= budget);
        // More runs than one per worker: tables spilled under pressure. The 160,000
        // distinct keys need about 18 MiB of table, more than the budget, and the
        // number of workers follows the budget, not the threads.
        // More runs than workers: some table spilled before the end of the pass.
        let runs = scratch.count("id dictionary run");
        let workers = build::worker_cap(budget);
        assert!(
            u64::try_from(runs).unwrap() > workers,
            "{runs} runs for {workers} workers"
        );
        assert!(big_runs >= 1);
        assert!(scratch.count("id dictionary order bucket") > 1);
        assert!(scratch.count("id dictionary scatter") > 1);
    }
}

#[test]
fn a_budget_that_cannot_hold_the_work_is_refused() {
    let keys = addresses(1000, 2000);
    let budget = Budget::new(4096);
    let source = Keys {
        column: ColumnType::Utf8,
        keys,
        chunk: 100,
    };
    let err =
        build_dictionary(&source, &BuildOptions::default(), &budget, &MemoryScratch).unwrap_err();
    assert!(matches!(err, DictionaryError::Budget(_)), "{err}");
    assert_eq!(budget.available(), budget.limit());
}

#[test]
fn reaching_2_to_the_32_nodes_is_refused_naming_the_issue() {
    assert!(check_node_count((1 << 32) - 1).is_ok());
    let err = check_node_count(1 << 32).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("#114") && message.contains("4294967296"),
        "{message}"
    );
}

#[test]
fn an_empty_input_gives_an_empty_dictionary() {
    let budget = Budget::new(1 << 24);
    for column in [ColumnType::Utf8, ColumnType::Int { bits: 32 }] {
        let dict = build(
            column,
            &[],
            BuildOptions::default(),
            &budget,
            &MemoryScratch,
        );
        assert!(dict.is_empty());
        assert_eq!(dict.lookup(Key::from("x")).unwrap(), None);
        assert_eq!(dict.lookup(Key::Int(0)).unwrap(), None);
        let again = reopen(&dict, &budget);
        assert!(again.is_empty());
    }
}

#[test]
fn renumbering_changes_ids_and_keeps_keys_and_graph_order() {
    let cases = [
        (ColumnType::Utf8, users(2000, 5000)),
        (ColumnType::Utf8, addresses(2000, 5000)),
        (
            ColumnType::UInt { bits: 64 },
            (0..5000).map(|i| In::UInt(splitmix(i) % 3000)).collect(),
        ),
    ];
    for (column, keys) in &cases {
        for layout in [Layout::Split, Layout::SlotOrdered] {
            let budget = Budget::new(1 << 30);
            let dict = build(
                *column,
                keys,
                BuildOptions::default().with_layout(layout),
                &budget,
                &MemoryScratch,
            );
            let n = dict.len();
            let perm: Vec<u32> = {
                let mut p: Vec<u32> = (0..n).collect();
                p.sort_by_key(|&i| splitmix(u64::from(i)));
                p
            };
            let once = dict.renumber(&perm, &MemoryScratch, &budget).unwrap();
            assert!(once.is_renumbered());
            let perm2: Vec<u32> = (0..n).map(|i| (i + 7) % n).collect();
            let twice = once.renumber(&perm2, &MemoryScratch, &budget).unwrap();
            let reopened = reopen(&twice, &budget);
            for old in 0..n {
                let key = dict.key(NodeId::new(old)).unwrap();
                let new = perm2[usize::try_from(perm[usize::try_from(old).unwrap()]).unwrap()];
                for d in [&twice, &reopened] {
                    assert_eq!(d.key(NodeId::new(new)).unwrap(), key);
                    assert_eq!(d.graph_position(NodeId::new(new)).unwrap(), old);
                }
                let k = keys
                    .iter()
                    .find(|k| dict.lookup(k.key()).unwrap() == Some(NodeId::new(old)))
                    .unwrap();
                assert_eq!(twice.lookup(k.key()).unwrap(), Some(NodeId::new(new)));
            }
            assert!(matches!(
                dict.renumber(
                    &vec![0; usize::try_from(n).unwrap()],
                    &MemoryScratch,
                    &budget
                ),
                Err(DictionaryError::InvalidPermutation)
            ));
            assert!(matches!(
                dict.renumber(&[0], &MemoryScratch, &budget),
                Err(DictionaryError::InvalidPermutation)
            ));
        }
    }
}

#[test]
fn prefix_and_range_search_agree_with_a_scan() {
    let keys = users(5000, 8000);
    let budget = Budget::new(1 << 30);
    let dict = build(
        ColumnType::Utf8,
        &keys,
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    let mut all: Vec<(Vec<u8>, NodeId)> = (0..dict.len())
        .map(|i| {
            let mut k = Vec::new();
            dict.normalized_key(NodeId::new(i), &mut k).unwrap();
            (k, NodeId::new(i))
        })
        .collect();
    all.sort();
    for prefix in [
        &b"https://github.com/user1"[..],
        b"https://t.me/",
        b"zzz",
        b"",
        b"https://twitter.com/user49",
    ] {
        let expected: Vec<NodeId> = all
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(_, id)| *id)
            .take(50)
            .collect();
        assert_eq!(
            dict.prefix_search(prefix, 50).unwrap(),
            expected,
            "{prefix:?}"
        );
    }
    let (lo, hi) = (
        &b"https://github.com/user2"[..],
        &b"https://github.com/user3"[..],
    );
    let expected: Vec<NodeId> = all
        .iter()
        .filter(|(k, _)| k.as_slice() >= lo && k.as_slice() < hi)
        .map(|(_, id)| *id)
        .take(1000)
        .collect();
    assert_eq!(dict.range_search(lo, hi, 1000).unwrap(), expected);
    assert!(dict.resident_bytes() > 0);
    let hex = build(
        ColumnType::Utf8,
        &addresses(10, 10),
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    assert!(matches!(
        hex.prefix_search(b"0x", 1),
        Err(DictionaryError::Unsupported(_))
    ));
}

/// Writes `value` as little-endian `u64`s over a section.
fn fill_u64(bytes: &mut [u8], from: usize, value: u64) {
    for c in bytes[from..].chunks_exact_mut(8) {
        c.copy_from_slice(&value.to_le_bytes());
    }
}

fn fill_u32(bytes: &mut [u8], value: u32) {
    for c in bytes.chunks_exact_mut(4) {
        c.copy_from_slice(&value.to_le_bytes());
    }
}

/// Every look-up of every key and every id, which must end in a value or an error,
/// never an abort, and never in an id at or above `len`.
fn probe<S: ByteSource>(dict: &Dictionary<'_, S>, keys: &[In]) -> Result<(), DictionaryError> {
    probe_lookups(dict, keys)?;
    // Renumbering reads every section too; then the renumbered dictionary is probed.
    let budget = Budget::new(1 << 28);
    let perm: Vec<u32> = (0..dict.len()).rev().collect();
    let renumbered = dict.renumber(&perm, &MemoryScratch, &budget)?;
    probe_lookups(&renumbered, keys)
}

fn probe_lookups<S: ByteSource>(
    dict: &Dictionary<'_, S>,
    keys: &[In],
) -> Result<(), DictionaryError> {
    for k in keys {
        for id in [dict.lookup(k.key())?, dict.id_of_member(k.key())?]
            .into_iter()
            .flatten()
        {
            assert!(
                id.get() < dict.len(),
                "id {id} for a dictionary of {}",
                dict.len()
            );
        }
    }
    let mut out = Vec::new();
    for id in 0..dict.len() {
        dict.normalized_key(NodeId::new(id), &mut out)?;
        assert!(dict.graph_position(NodeId::new(id))? < dict.len());
    }
    Ok(())
}

/// Hostile files: offsets of 2^40, a truncated key store, ids of
/// `u32::MAX` in every array of ids, a remap outside its part, a huge header. Each is
/// refused when opened or by the look-up that meets it, with an error.
#[test]
fn hostile_sections_are_refused_without_aborting() {
    let budget = Budget::new(1 << 30);
    let utf8 = ColumnType::Utf8;
    let keys = users(100, 300);
    let split = build(
        utf8,
        &keys,
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    let slot = build(
        utf8,
        &keys,
        BuildOptions::default().with_layout(Layout::SlotOrdered),
        &budget,
        &MemoryScratch,
    );
    let ints: Vec<In> = (0..300).map(|i| In::UInt(i % 100)).collect();
    let dense = build(
        ColumnType::UInt { bits: 64 },
        &ints,
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    let perm: Vec<u32> = (0..split.len()).rev().collect();
    let renumbered = split.renumber(&perm, &MemoryScratch, &budget).unwrap();
    let dense_perm: Vec<u32> = (0..dense.len()).rev().collect();
    let dense_renumbered = dense
        .renumber(&dense_perm, &MemoryScratch, &budget)
        .unwrap();

    let refused = |dict: &Dictionary<'_, MemorySource<Vec<u8>>>,
                   keys: &[In],
                   edit: &dyn Fn(&mut Sections)| {
        let mut bytes: Sections = section_bytes(dict).into_iter().collect();
        edit(&mut bytes);
        let result = open_bytes(&bytes, &budget).and_then(|d| probe(&d, keys));
        assert!(
            matches!(result, Err(DictionaryError::Corrupt(_))),
            "{result:?}"
        );
    };
    // Offsets past their section (2^40), and a key store cut in half.
    refused(&split, &keys, &|b| {
        fill_u64(b.get_mut("ids.key_offsets").unwrap(), 8, 1 << 40);
    });
    refused(&slot, &keys, &|b| {
        fill_u64(b.get_mut("ids.record_offsets").unwrap(), 8, 1 << 40);
    });
    refused(&split, &keys, &|b| {
        let k = b.get_mut("ids.keys").unwrap();
        k.truncate(k.len() / 2);
    });
    // A record shorter than its id (two equal offsets): refused by the look-up and by
    // `renumber`, which must not panic.
    refused(&slot, &keys, &|b| {
        let o = b.get_mut("ids.record_offsets").unwrap();
        let first: [u8; 8] = o[0..8].try_into().unwrap();
        o[8..16].copy_from_slice(&first);
    });
    {
        let mut bytes: Sections = section_bytes(&slot).into_iter().collect();
        let o = bytes.get_mut("ids.record_offsets").unwrap();
        let first: [u8; 8] = o[0..8].try_into().unwrap();
        o[8..16].copy_from_slice(&first);
        let opened = open_bytes(&bytes, &budget).unwrap();
        let perm: Vec<u32> = (0..opened.len()).collect();
        assert!(matches!(
            opened.renumber(&perm, &MemoryScratch, &budget),
            Err(DictionaryError::Corrupt(_))
        ));
    }
    // Ids of u32::MAX.
    refused(&split, &keys, &|b| {
        fill_u32(b.get_mut("ids.slot_to_id").unwrap(), u32::MAX);
    });
    refused(&slot, &keys, &|b| {
        // The id of every record is its first 4 bytes; spoil them all.
        let o = b.get("ids.record_offsets").unwrap().clone();
        let r = b.get_mut("ids.records").unwrap();
        for c in o.chunks_exact(8).take(o.len() / 8 - 1) {
            let at = usize::try_from(u64::from_le_bytes(c.try_into().unwrap())).unwrap();
            r[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        }
    });
    refused(&slot, &keys, &|b| {
        fill_u32(b.get_mut("ids.id_to_slot").unwrap(), u32::MAX);
    });
    refused(&renumbered, &keys, &|b| {
        fill_u32(b.get_mut("ids.order").unwrap(), u32::MAX);
    });
    refused(&dense_renumbered, &ints, &|b| {
        fill_u32(b.get_mut("ids.rank_to_id").unwrap(), u32::MAX);
    });
    refused(&dense_renumbered, &ints, &|b| {
        fill_u32(b.get_mut("ids.id_to_rank").unwrap(), u32::MAX);
    });
    // A remap entry outside its part.
    let big = build(
        utf8,
        &users(5000, 5000),
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    refused(&big, &[], &|b| {
        fill_u32(b.get_mut("ids.remap").unwrap(), u32::MAX);
    });
    // A header of 64 MiB is refused by its length, before it is read.
    refused(&split, &keys, &|b| {
        b.get_mut("ids.header").unwrap().resize(64 << 20, 0);
    });
    let before = budget.available();
    refused(&split, &keys, &|b| {
        b.get_mut("ids.pilots").unwrap().push(0);
    });
    assert_eq!(budget.available(), before);
}

#[test]
fn damaged_sections_are_refused() {
    let budget = Budget::new(1 << 28);
    let dict = build(
        ColumnType::Utf8,
        &users(100, 200),
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    let bytes: Sections = section_bytes(&dict).into_iter().collect();
    let open = |edit: &dyn Fn(&mut Sections)| {
        let mut b = bytes.clone();
        edit(&mut b);
        open_bytes(&b, &budget).map(|_| ())
    };
    assert!(open(&|_| {}).is_ok());
    assert!(matches!(
        open(&|b| {
            b.remove("ids.keys");
        }),
        Err(DictionaryError::Corrupt(_))
    ));
    assert!(matches!(
        open(&|b| {
            b.get_mut("ids.slot_to_id").unwrap().pop();
        }),
        Err(DictionaryError::Corrupt(_))
    ));
    assert!(matches!(
        open(&|b| {
            b.get_mut("ids.header").unwrap()[0] = b'X';
        }),
        Err(DictionaryError::Corrupt(_))
    ));
    assert!(matches!(
        open(&|b| {
            b.get_mut("ids.header").unwrap()[60] = 1;
        }),
        Err(DictionaryError::Corrupt(_))
    ));
    assert!(matches!(
        open(&|b| {
            b.insert("ids.order", vec![0; 4]);
        }),
        Err(DictionaryError::Corrupt(_))
    ));
    assert!(matches!(
        open(&|b| {
            b.get_mut("ids.parts").unwrap()[4] ^= 1;
        }),
        Err(DictionaryError::Corrupt(_))
    ));
    assert_eq!(budget.available(), budget.limit() - dict.resident_bytes());
}

/// Sections that open but give two ids each other's keys: only `verify` reads every
/// key, so only it sees that ids and keys are no longer a bijection.
#[test]
fn verify_finds_ids_and_keys_that_are_not_a_bijection() {
    let budget = Budget::new(1 << 28);
    for (layout, section) in [
        (Layout::Split, "ids.slot_to_id"),
        (Layout::SlotOrdered, "ids.id_to_slot"),
    ] {
        let dict = build(
            ColumnType::Utf8,
            &users(100, 200),
            BuildOptions::default().with_layout(layout),
            &budget,
            &MemoryScratch,
        );
        let mut bytes: Sections = section_bytes(&dict).into_iter().collect();
        open_bytes(&bytes, &budget).unwrap().verify().unwrap();
        let ids = bytes.get_mut(section).unwrap();
        let (first, second) = ids.split_at_mut(4);
        first.swap_with_slice(&mut second[..4]);
        let swapped = open_bytes(&bytes, &budget).unwrap();
        assert!(
            matches!(swapped.verify(), Err(DictionaryError::Corrupt(_))),
            "{layout:?}"
        );
    }
}

/// A digest of every section of builds that spill, printed for
/// [`ids_do_not_depend_on_the_thread_count`], which runs it in child processes with
/// different thread counts. It checks there that the small budget spilled.
#[test]
#[ignore = "run by ids_do_not_depend_on_the_thread_count in a child process"]
fn digest_probe() {
    let keys = addresses(160_000, 200_000);
    let mut digest = 0_u64;
    for layout in [Layout::Split, Layout::SlotOrdered] {
        for budget in [12 << 20, 1 << 30] {
            let budget = Budget::new(budget);
            let scratch = CountingScratch::default();
            let dict = build(
                ColumnType::Utf8,
                &keys,
                BuildOptions::default().with_layout(layout),
                &budget,
                &scratch,
            );
            if budget.limit() < 1 << 24 {
                let runs = u64::try_from(scratch.count("id dictionary run")).unwrap();
                assert!(
                    runs > build::worker_cap(budget.limit()),
                    "no spill at {}",
                    budget.limit()
                );
            }
            for (name, bytes) in section_bytes(&dict) {
                digest = splitmix(
                    digest
                        ^ xxhash_rust::xxh3::xxh3_64(name.as_bytes())
                        ^ xxhash_rust::xxh3::xxh3_64(&bytes),
                );
            }
        }
    }
    let threads = std::env::var("RAYON_NUM_THREADS").unwrap_or_default();
    #[expect(
        clippy::print_stdout,
        reason = "the parent test reads the digest from the output"
    )]
    {
        println!("digest threads={threads} value={digest:016x}");
    }
}

/// Builds succeed, and give the same bytes, at 1, 16 and 128 threads under a 12 MiB
/// budget.
#[test]
fn ids_do_not_depend_on_the_thread_count() {
    let exe = std::env::current_exe().unwrap();
    let mut digests = Vec::new();
    for threads in ["1", "16", "128"] {
        let out = std::process::Command::new(&exe)
            .args([
                "--exact",
                "ids::tests::digest_probe",
                "--ignored",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .env("RAYON_NUM_THREADS", threads)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let line = stdout
            .lines()
            .find_map(|l| l.find("digest threads=").map(|at| &l[at..]))
            .unwrap_or_else(|| panic!("no digest in {stdout}"));
        assert!(line.contains(&format!("threads={threads} ")), "{line}");
        digests.push(line.rsplit("value=").next().unwrap().to_owned());
    }
    assert!(digests.windows(2).all(|w| w[0] == w[1]), "{digests:?}");
}

/// A thread that holds a worker and asks for another (a reader that would nest a
/// parallel call) never waits for itself: it gets one more from the budget, or a
/// `Budget` error.
#[test]
fn a_thread_holding_a_worker_is_not_made_to_wait_for_itself() {
    let budget = Budget::new(4 << 20);
    let options = BuildOptions::default();
    let c = Collector::new(
        ColumnType::Utf8,
        KeyKind::Bytes,
        &options,
        &budget,
        &MemoryScratch,
    )
    .unwrap();
    let first = c.worker().unwrap();
    // One worker at this budget: the second comes from what is left of the budget.
    let second = c.worker().unwrap();
    let mut held = vec![first, second];
    let refused = loop {
        match c.worker() {
            Ok(w) => held.push(w),
            Err(e) => break e,
        }
    };
    assert!(matches!(refused, DictionaryError::Budget(_)));
    drop(held);
    // Back on another thread, nothing is held: a worker is idle again.
    std::thread::scope(|s| {
        s.spawn(|| drop(c.worker().unwrap()));
    });
}

/// The input of the threshold tests.
fn threshold_input() -> Vec<In> {
    addresses(60_000, 80_000)
}

/// Whether the threshold input builds under `budget` (bytes), in this process.
fn builds_under(budget: u64, layout: Layout) -> bool {
    let source = Keys {
        column: ColumnType::Utf8,
        keys: threshold_input(),
        chunk: 97,
    };
    let budget = Budget::new(budget);
    let options = BuildOptions::default().with_layout(layout);
    match build_dictionary(&source, &options, &budget, &MemoryScratch) {
        Ok(_) => true,
        Err(DictionaryError::Budget(_)) => false,
        Err(e) => panic!("{e}"),
    }
}

/// One build under `EVERYGRAPH_PROBE_BUDGET`, for [`success_at_the_threshold_does_not_depend_on_threads_or_runs`].
#[test]
#[ignore = "run by success_at_the_threshold_does_not_depend_on_threads_or_runs in a child process"]
fn threshold_probe() {
    let budget: u64 = std::env::var("EVERYGRAPH_PROBE_BUDGET")
        .unwrap()
        .parse()
        .unwrap();
    let layout = if std::env::var("EVERYGRAPH_PROBE_SLOT").is_ok() {
        Layout::SlotOrdered
    } else {
        Layout::Split
    };
    let ok = builds_under(budget, layout);
    #[expect(
        clippy::print_stdout,
        reason = "the parent test reads the outcome from the output"
    )]
    {
        println!("outcome={}", if ok { "ok" } else { "refused" });
    }
}

/// Whether a build fits a budget is a function of the budget alone: just below the
/// smallest budget that works, every run at 1, 16 and 128 threads is refused, and at
/// it every run succeeds.
#[test]
fn success_at_the_threshold_does_not_depend_on_threads_or_runs() {
    for layout in [Layout::Split, Layout::SlotOrdered] {
        // The smallest budget that works, to 1 KiB, in this process.
        let (mut lo, mut hi) = (1_u64 << 20, 64_u64 << 20);
        assert!(!builds_under(lo, layout) && builds_under(hi, layout));
        while hi - lo > 1024 {
            let mid = lo + (hi - lo) / 2;
            if builds_under(mid, layout) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let exe = std::env::current_exe().unwrap();
        for threads in ["1", "16", "128"] {
            for _ in 0..3 {
                for (budget, expected) in [(lo, "refused"), (hi, "ok")] {
                    let mut command = std::process::Command::new(&exe);
                    command
                        .args([
                            "--exact",
                            "ids::tests::threshold_probe",
                            "--ignored",
                            "--nocapture",
                            "--test-threads",
                            "1",
                        ])
                        .env("RAYON_NUM_THREADS", threads)
                        .env("EVERYGRAPH_PROBE_BUDGET", budget.to_string());
                    if layout == Layout::SlotOrdered {
                        command.env("EVERYGRAPH_PROBE_SLOT", "1");
                    }
                    let out = command.output().unwrap();
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    assert!(
                        out.status.success(),
                        "{stdout}\n{}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                    let outcome = stdout
                        .lines()
                        .find_map(|l| l.find("outcome=").map(|at| &l[at + 8..]))
                        .unwrap_or_else(|| panic!("no outcome in {stdout}"));
                    assert_eq!(
                        outcome.trim(),
                        expected,
                        "{layout:?}, {threads} threads, budget {budget}"
                    );
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    /// Small random inputs of every shape, both layouts: ids as the reference says.
    #[test]
    fn random_inputs_match_the_reference(
        raw in proptest::collection::vec((0_u64..400, 0_u8..4), 0..600),
        text_keys in any::<bool>(),
        order in prop_oneof![Just(IdOrder::Natural), Just(IdOrder::FirstAppearance), Just(IdOrder::SortedKey)],
        layout in prop_oneof![Just(Layout::Split), Just(Layout::SlotOrdered)],
    ) {
        let (column, keys): (ColumnType, Vec<In>) = if text_keys {
            (ColumnType::Utf8, raw.iter().map(|&(v, f)| text("k".repeat(usize::from(f) + 1) + &v.to_string())).collect())
        } else {
            (ColumnType::Int { bits: 64 }, raw.iter().map(|&(v, f)| In::Int(i64::try_from(v).unwrap() - if f == 0 { 1000 } else { 0 })).collect())
        };
        let budget = Budget::new(1 << 28);
        let dict = build(column, &keys, BuildOptions::default().with_order(order).with_layout(layout), &budget, &MemoryScratch);
        check(&dict, column, &keys, order);
    }

    /// A cheap fuzz target: random bytes changed anywhere in the sections of small
    /// dictionaries of every layout. Opening and every look-up end in a value or an
    /// error — no panic, no abort — and an id found is always below `len`.
    #[test]
    fn opening_and_looking_up_damaged_sections_never_panics(
        case in 0_usize..8,
        edits in proptest::collection::vec((0_usize..14, any::<u32>(), any::<u8>(), 0_u8..3), 1..8),
    ) {
        let budget = Budget::new(1 << 28);
        let utf8 = ColumnType::Utf8;
        let ints: Vec<In> = (0..120).map(|i| In::UInt(i % 40)).collect();
        let keys = users(40, 120);
        let slot = BuildOptions::default().with_layout(Layout::SlotOrdered);
        let (column, input, options) = match case % 4 {
            0 => (utf8, keys, BuildOptions::default()),
            1 => (utf8, keys, slot),
            2 => (utf8, addresses(40, 120), slot),
            _ => (ColumnType::UInt { bits: 64 }, ints, BuildOptions::default()),
        };
        let built = build(column, &input, options, &budget, &MemoryScratch);
        // Cases 4 to 7: the same, renumbered.
        let dict = if case >= 4 {
            let perm: Vec<u32> = (0..built.len()).map(|i| (i * 7 + 3) % built.len()).collect();
            built.renumber(&perm, &MemoryScratch, &budget).unwrap()
        } else {
            built
        };
        let mut bytes: Sections = section_bytes(&dict).into_iter().collect();
        let names: Vec<&'static str> = bytes.keys().copied().collect();
        for (section, at, value, mode) in edits {
            let name = names[section % names.len()];
            let b = bytes.get_mut(name).unwrap();
            if b.is_empty() {
                continue;
            }
            let at = usize::try_from(at).unwrap() % b.len();
            match mode {
                // A byte.
                0 => b[at] = value,
                // An aligned u32 set to a small value.
                1 if b.len() >= 4 => {
                    let at = (at / 4 * 4).min(b.len() - 4);
                    b[at..at + 4].copy_from_slice(&u32::from(value % 8).to_le_bytes());
                }
                // A u64 copied onto its neighbour (two equal offsets).
                _ if b.len() >= 16 => {
                    let at = (at / 8 * 8).min(b.len() - 16);
                    let word: [u8; 8] = b[at..at + 8].try_into().unwrap();
                    b[at + 8..at + 16].copy_from_slice(&word);
                }
                _ => b[at] = value,
            }
        }
        if let Ok(d) = open_bytes(&bytes, &budget) {
            let _ = probe(&d, &input);
        }
    }
}
