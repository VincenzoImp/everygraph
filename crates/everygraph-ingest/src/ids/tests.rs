// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests of the whole dictionary against a reference computed the obvious way.

use std::collections::BTreeMap;
use std::io;
use std::sync::Mutex;

use everygraph_core::{ByteSource, MemoryAccountant, MemorySource, NodeId};
use proptest::prelude::*;

use super::kind::MAX_FIXED_WIDTH;
use super::test_support::Budget;
use super::*;

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

/// A scratch in memory that counts the runs the distinct-key pass spilled.
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

/// The ids the dictionary must assign, computed the obvious way: normalized key ->
/// (id, an original spelling).
fn reference(
    kind: KeyKind,
    column: ColumnType,
    keys: &[In],
    order: IdOrder,
) -> BTreeMap<Vec<u8>, u32> {
    let mut first: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
    for (pos, k) in keys.iter().enumerate() {
        let mut buf = [0; MAX_FIXED_WIDTH];
        let norm = kind
            .normalize(column, k.key(), &mut buf)
            .expect("the kind accepts every key")
            .to_vec();
        first.entry(norm).or_insert(pos);
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
fn check<S: ByteSource>(dict: &Dictionary<'_, S>, column: ColumnType, keys: &[In], order: IdOrder) {
    let kind = dict.kind();
    let expected = reference(kind, column, keys, order);
    assert_eq!(
        u64::from(dict.len()),
        u64::try_from(expected.len()).unwrap()
    );
    for k in keys {
        let mut buf = [0; MAX_FIXED_WIDTH];
        let norm = kind.normalize(column, k.key(), &mut buf).unwrap();
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
}

fn build<'a, Sc: Scratch>(
    column: ColumnType,
    keys: &[In],
    options: BuildOptions,
    budget: &'a Budget,
    scratch: &'a Sc,
) -> Dictionary<'a, Sc::Source> {
    let source = Keys {
        column,
        keys: keys.to_vec(),
        chunk: 97,
    };
    build_dictionary(&source, &options, budget, scratch).unwrap()
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

fn reopen<'a>(
    dict: &Dictionary<'_, MemorySource<Vec<u8>>>,
    budget: &'a Budget,
) -> Dictionary<'a, MemorySource<Vec<u8>>> {
    let bytes: BTreeMap<&str, Vec<u8>> = section_bytes(dict).into_iter().collect();
    Dictionary::open(budget, |name| {
        bytes.get(name).map(|b| MemorySource::new(b.clone()))
    })
    .unwrap()
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
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

#[test]
fn every_kind_order_and_layout_matches_the_reference_and_survives_reopening() {
    let utf8 = ColumnType::Utf8;
    let cases: Vec<(ColumnType, Vec<In>, KeyKind)> = vec![
        (
            utf8,
            addresses(3000, 9000),
            KeyKind::Hex {
                prefix: HexPrefix::Lower,
                digits: 40,
            },
        ),
        (utf8, users(3000, 9000), KeyKind::Bytes),
        (
            ColumnType::Int { bits: 64 },
            (0..5000)
                .map(|i| In::Int(i64::try_from(splitmix(i) % 4000).unwrap()))
                .collect(),
            KeyKind::Integers { dense: true },
        ),
        (
            ColumnType::Int { bits: 64 },
            (0..5000)
                .map(|i| In::Int(i64::from_ne_bytes(splitmix(i % 1500).to_ne_bytes())))
                .collect(),
            KeyKind::Integers { dense: false },
        ),
        (
            ColumnType::UInt { bits: 32 },
            (0..5000).map(|i| In::UInt(splitmix(i) % 100)).collect(),
            KeyKind::Integers { dense: true },
        ),
        (
            utf8,
            (0..3000)
                .map(|i| text((splitmix(i) % 2000).to_string()))
                .collect(),
            KeyKind::Integers { dense: true },
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
            ColumnType::Binary,
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
                let dict = build(*column, keys, options, &budget, &MemoryScratch);
                assert_eq!(dict.kind(), *kind);
                assert_eq!(dict.order(), order.resolve(*kind));
                check(&dict, *column, keys, order);
                let again = reopen(&dict, &budget);
                check(&again, *column, keys, order);
                assert_eq!(section_bytes(&again), section_bytes(&dict));
                drop((dict, again));
                assert_eq!(budget.available(), budget.total(), "everything released");
            }
        }
    }
}

#[test]
fn hexadecimal_keys_are_found_in_any_case_and_come_back_lowercase() {
    let keys = vec![
        text("0xAbCdEf0123456789abcdef0123456789ABCDEF01"),
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
        .lookup(Key::from("0XABCDEF0123456789ABCDEF0123456789ABCDEF01"))
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

#[test]
fn a_key_that_breaks_the_sampled_kind_widens_it() {
    let mut keys = addresses(5000, 5000);
    keys.push(text("not-an-address"));
    keys.extend(addresses(5000, 100));
    let budget = Budget::new(1 << 28);
    let dict = build(
        ColumnType::Utf8,
        &keys,
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    assert_eq!(dict.kind(), KeyKind::Bytes);
    check(&dict, ColumnType::Utf8, &keys, IdOrder::Natural);
    // As text, the mixed-case spellings are different keys.
    let ints: Vec<In> = (0..300)
        .map(|i| In::UInt(if i == 299 { 1 << 40 } else { i }))
        .collect();
    let dict = build(
        ColumnType::UInt { bits: 64 },
        &ints,
        BuildOptions::default(),
        &budget,
        &MemoryScratch,
    );
    assert_eq!(dict.kind(), KeyKind::Integers { dense: false });
    check(
        &dict,
        ColumnType::UInt { bits: 64 },
        &ints,
        IdOrder::Natural,
    );
}

/// The section bytes of a build — and so every id — are the same under any budget,
/// and a small budget really spills.
#[test]
fn ids_do_not_depend_on_the_budget_and_a_small_budget_spills() {
    let utf8 = ColumnType::Utf8;
    for (keys, order) in [
        (addresses(60_000, 150_000), IdOrder::FirstAppearance),
        (users(60_000, 150_000), IdOrder::SortedKey),
        (users(60_000, 150_000), IdOrder::FirstAppearance),
    ] {
        for layout in [Layout::Split, Layout::SlotOrdered] {
            let options = BuildOptions::default()
                .with_order(order)
                .with_layout(layout);
            let big = Budget::new(1 << 32);
            let big_scratch = CountingScratch::default();
            let reference = section_bytes(&build(utf8, &keys, options, &big, &big_scratch));
            let big_runs = big_scratch.count("id dictionary run");
            for budget in [12 << 20, 32 << 20, 1 << 30] {
                let small = Budget::new(budget);
                let scratch = CountingScratch::default();
                let dict = build(utf8, &keys, options, &small, &scratch);
                check(&dict, utf8, &keys, order);
                assert_eq!(section_bytes(&dict), reference, "budget {budget}");
                assert!(small.peak() <= budget);
                if budget < 1 << 24 {
                    // More runs than one per worker: tables spilled under pressure.
                    let runs = scratch.count("id dictionary run");
                    assert!(
                        runs > big_runs,
                        "budget {budget}: {runs} runs, {big_runs} without pressure"
                    );
                    assert!(scratch.count("id dictionary order bucket") > 1);
                    assert!(scratch.count("id dictionary scatter") > 1);
                }
            }
        }
    }
}

#[test]
fn a_budget_that_cannot_hold_the_work_is_refused_before_allocating() {
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
    assert_eq!(budget.available(), budget.total());
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
        Dictionary::open(&budget, |name| {
            b.get(name).map(|v| MemorySource::new(v.clone()))
        })
        .map(|_| ())
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
    assert_eq!(budget.available(), budget.total() - dict.resident_bytes());
}

/// A digest of every section of a build that exercises spilling, printed for
/// [`ids_do_not_depend_on_the_thread_count`], which runs it in child processes with
/// different thread counts.
#[test]
#[ignore = "run by ids_do_not_depend_on_the_thread_count in a child process"]
fn digest_probe() {
    let keys = addresses(40_000, 100_000);
    let mut digest = 0_u64;
    for layout in [Layout::Split, Layout::SlotOrdered] {
        for budget in [12 << 20, 1 << 30] {
            let budget = Budget::new(budget);
            let dict = build(
                ColumnType::Utf8,
                &keys,
                BuildOptions::default().with_layout(layout),
                &budget,
                &MemoryScratch,
            );
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

#[test]
fn ids_do_not_depend_on_the_thread_count() {
    let exe = std::env::current_exe().unwrap();
    let mut digests = Vec::new();
    for threads in ["1", "3", "8"] {
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
}
