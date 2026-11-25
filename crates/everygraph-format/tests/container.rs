// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests of the `.eg` container (`docs/spec/eg-container.md`): round trips, the crash
//! guarantees of the update protocol under injected faults, forward compatibility,
//! damage detection, and the cost of opening.

#![forbid(unsafe_code)]

use std::cell::Cell;
use std::collections::BTreeMap;

use everygraph_core::{ByteSink, ByteSource, MemorySource, ReadError, WriteError};
use everygraph_format::container::{
    Codec, Container, ContainerError, Entry, Input, Kind, Provenance, Writer, compact,
};
use proptest::prelude::*;

type Mem = MemorySource<Vec<u8>>;

fn kind(code: u32) -> Kind {
    Kind::new(code).unwrap()
}

/// Deterministic bytes for section `seed` of length `len`.
fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()[0]
        })
        .collect()
}

fn record(operation: &str) -> Provenance {
    Provenance {
        engine: "everygraph test".into(),
        operation: operation.into(),
        started: Some(1),
        finished: Some(2),
        inputs: vec![Input {
            name: "edges.csv".into(),
            bytes: 10,
            modified: None,
            algorithm: "blake3".into(),
            digest: vec![9; 32],
        }],
        parameters: [("seed".to_owned(), "42".to_owned())].into_iter().collect(),
    }
}

/// The content of every section of a container, by name.
fn contents<S: ByteSource>(file: &Container<S>) -> BTreeMap<String, Vec<u8>> {
    file.entries()
        .iter()
        .map(|e| {
            let section = file.section(&e.name).unwrap();
            let mut buf = vec![0; usize::try_from(section.len()).unwrap()];
            section.read_at(0, &mut buf).unwrap();
            (e.name.clone(), buf)
        })
        .collect()
}

/// A small graph-like file: a provenance record, two sections that refer to it, one
/// of an unknown kind, one empty.
fn base_file() -> Vec<u8> {
    let mut w = Writer::create(Mem::default()).unwrap();
    w.add_provenance("provenance/ingest", &record("ingest"))
        .unwrap();
    w.add_section(
        "adjacency.out.index",
        Kind::ADJACENCY_INDEX,
        &bytes(1, 300),
        Some("provenance/ingest"),
    )
    .unwrap();
    w.add_section(
        "adjacency.out.data",
        Kind::ADJACENCY_DATA,
        &bytes(2, 5000),
        Some("provenance/ingest"),
    )
    .unwrap();
    w.add_section("x.unknown", kind(0x8000_0007), &bytes(3, 17), None)
        .unwrap();
    w.add_section("ids.header", Kind::ID_DICTIONARY, &[], None)
        .unwrap();
    assert_eq!(w.commit().unwrap(), 1);
    w.into_sink().into_inner()
}

#[test]
fn a_written_file_reads_back_with_its_provenance() {
    let file = Container::open(Mem::new(base_file())).unwrap();
    assert_eq!(file.generation(), 1);
    assert_eq!(file.version(), (1, 0));
    assert_eq!(file.recovered_from(), None);
    let names: Vec<&str> = file.entries().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "adjacency.out.data",
            "adjacency.out.index",
            "ids.header",
            "provenance/ingest",
            "x.unknown"
        ]
    );
    file.verify_all().unwrap();
    let c = contents(&file);
    assert_eq!(c["adjacency.out.data"], bytes(2, 5000));
    assert_eq!(c["ids.header"], b"");
    assert_eq!(
        file.provenance("adjacency.out.index").unwrap(),
        Some(record("ingest"))
    );
    assert_eq!(file.provenance("x.unknown").unwrap(), None);
    // Sections are block-aligned, with at least 16 zero bytes after each.
    for e in file.entries() {
        assert_eq!(e.offset % 4096, 0);
        assert!(e.offset >= 8192);
    }
    assert!(!file.get("x.unknown").unwrap().kind.is_known());
    assert_eq!(file.reclaimable_bytes(), 0);
    assert_eq!(file.live_bytes(), file.source().len());
    // A zero-copy view of a section of an in-memory file.
    let data = file.section("adjacency.out.data").unwrap();
    assert_eq!(data.as_slice().unwrap(), &bytes(2, 5000)[..]);
}

#[test]
fn an_update_appends_and_never_changes_a_byte_the_file_held() {
    let before = base_file();
    let mut w = Writer::update(Mem::new(before.clone())).unwrap();
    w.add_provenance("provenance/pagerank", &record("pagerank"))
        .unwrap();
    w.add_section(
        "results/pagerank",
        kind(0x8000_0001),
        &bytes(4, 9000),
        Some("provenance/pagerank"),
    )
    .unwrap();
    w.remove("x.unknown").unwrap();
    assert_eq!(w.commit().unwrap(), 2);
    let after = w.into_sink().into_inner();
    // Only the header block that did not hold the last commit changed below the old end.
    let header_block_1 = 4096..8192;
    for (i, (a, b)) in before.iter().zip(&after).enumerate() {
        assert!(a == b || header_block_1.contains(&i), "byte {i} changed");
    }
    let file = Container::open(Mem::new(after.clone())).unwrap();
    assert_eq!(file.generation(), 2);
    assert!(file.get("x.unknown").is_none());
    file.verify_all().unwrap();
    assert!(file.reclaimable_bytes() > 0);

    // The next commit goes to block 0 again, and so on.
    let mut w = Writer::update(Mem::new(after)).unwrap();
    w.remove("results/pagerank").unwrap();
    w.remove("provenance/pagerank").unwrap();
    assert_eq!(w.commit().unwrap(), 3);
    let file = Container::open(w.into_sink()).unwrap();
    assert_eq!(file.generation(), 3);
    assert_eq!(file.entries().len(), 4);
}

#[test]
fn compaction_keeps_the_live_sections_and_drops_everything_else() {
    let mut w = Writer::update(Mem::new(base_file())).unwrap();
    w.remove("adjacency.out.data").unwrap();
    w.add_section(
        "adjacency.out.data",
        Kind::ADJACENCY_DATA,
        &bytes(5, 7000),
        Some("provenance/ingest"),
    )
    .unwrap();
    w.commit().unwrap();
    // A section appended but never committed, left behind by a "crash".
    w.add_section("abandoned", kind(0x8000_0002), &bytes(6, 3000), None)
        .unwrap();
    let grown = w.into_sink();
    let file = Container::open(grown).unwrap();
    let expected = contents(&file);
    let reclaimable = file.reclaimable_bytes();
    assert!(reclaimable >= 3000 + 5000);

    let compacted = compact(&file, Mem::default()).unwrap();
    let small = Container::open(compacted).unwrap();
    assert_eq!(small.generation(), 1);
    assert_eq!(contents(&small), expected);
    small.verify_all().unwrap();
    assert_eq!(small.reclaimable_bytes(), 0);
    assert_eq!(
        small.source().len() + reclaimable,
        file.source().len(),
        "compaction reclaims exactly what reclaimable_bytes reports"
    );
    let hashes = |c: &Container<Mem>| {
        c.entries()
            .iter()
            .map(|e| (e.name.clone(), e.hash, e.kind, e.provenance.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(hashes(&small), hashes(&file));
}

#[test]
fn the_writer_refuses_what_the_specification_forbids() {
    let mut w = Writer::update(Mem::new(base_file())).unwrap();
    let k = kind(0x8000_0000);
    assert!(matches!(
        w.add_section("ids.header", k, b"", None),
        Err(ContainerError::NameTaken(_))
    ));
    assert!(matches!(
        w.add_section("", k, b"", None),
        Err(ContainerError::InvalidName)
    ));
    assert!(matches!(
        w.add_section(&"n".repeat(1025), k, b"", None),
        Err(ContainerError::InvalidName)
    ));
    assert!(w.add_section(&"n".repeat(1024), k, b"", None).is_ok());
    assert!(matches!(
        w.add_section("a", k, b"", Some("nope")),
        Err(ContainerError::NoSuchSection(_))
    ));
    assert!(matches!(
        w.add_section("a", k, b"", Some("x.unknown")),
        Err(ContainerError::NotProvenance(_))
    ));
    assert!(matches!(
        w.add_section("a", Kind::PROVENANCE, b"", None),
        Err(ContainerError::NotProvenance(_))
    ));
    assert!(matches!(
        w.remove("provenance/ingest"),
        Err(ContainerError::StillReferenced { .. })
    ));
    assert!(matches!(
        w.remove("nope"),
        Err(ContainerError::NoSuchSection(_))
    ));
    assert_eq!(Kind::new(0), None);
    // Nothing above changed what the file holds except the one valid section.
    assert_eq!(w.commit().unwrap(), 2);
    // A commit with nothing new writes nothing.
    let len = w.sink().len();
    assert_eq!(w.commit().unwrap(), 2);
    assert_eq!(w.sink().len(), len);
}

#[test]
fn an_empty_container_is_valid() {
    let mut w = Writer::create(Mem::default()).unwrap();
    w.commit().unwrap();
    let file = Container::open(w.into_sink()).unwrap();
    assert!(file.entries().is_empty());
    assert_eq!(file.source().len(), 8192 + 4096);
}

#[test]
fn what_is_not_a_container_is_refused_plainly() {
    assert!(matches!(
        Container::open(Mem::new(Vec::new())),
        Err(ContainerError::NotAContainer)
    ));
    assert!(matches!(
        Container::open(Mem::new(vec![0; 20_000])),
        Err(ContainerError::NotAContainer)
    ));
    // Created, never committed.
    let w = Writer::create(Mem::default()).unwrap();
    assert!(matches!(
        Container::open(w.into_sink()),
        Err(ContainerError::NotAContainer)
    ));
}

// ---------------------------------------------------------------------------------
// Header patching, to make files that a later version or a damaged disk would make.

/// Rewrites header block `block` with `edit` and fixes its hash.
fn patch_header(file: &mut [u8], block: usize, edit: impl FnOnce(&mut [u8])) {
    let b = &mut file[block * 4096..block * 4096 + 128];
    edit(b);
    let own = blake3::hash(&b[..96]);
    b[96..128].copy_from_slice(own.as_bytes());
}

/// The block holding the newest commit.
fn newest_block(file: &[u8]) -> usize {
    let generation = |i: usize| {
        let b = &file[i * 4096..i * 4096 + 128];
        if b[..8] == *b"\x89EGRAPH\n" && blake3::hash(&b[..96]).as_bytes()[..] == b[96..128] {
            u64::from_le_bytes(b[24..32].try_into().unwrap())
        } else {
            0
        }
    };
    usize::from(generation(1) > generation(0))
}

/// Rewrites the newest table of contents with `edit` and fixes its hash and the
/// header's.
fn patch_toc(file: &mut [u8], edit: impl FnOnce(&mut [u8])) {
    let block = newest_block(file);
    let h = &file[block * 4096..];
    let at = usize::try_from(u64::from_le_bytes(h[48..56].try_into().unwrap())).unwrap();
    let len = usize::try_from(u64::from_le_bytes(h[56..64].try_into().unwrap())).unwrap();
    edit(&mut file[at..at + len]);
    let hash = *blake3::hash(&file[at..at + len]).as_bytes();
    patch_header(file, block, |b| b[64..96].copy_from_slice(&hash));
}

#[test]
fn a_later_minor_version_is_read_but_not_updated() {
    let mut file = base_file();
    patch_header(&mut file, 0, |b| b[14] = 7);
    let opened = Container::open(Mem::new(file.clone())).unwrap();
    assert_eq!(opened.version(), (1, 7));
    opened.verify_all().unwrap();
    assert!(matches!(
        Writer::update(Mem::new(file)),
        Err(ContainerError::NewerMinor { minor: 7 })
    ));
}

#[test]
fn unknown_required_features_and_versions_are_refused_unknown_optional_ones_kept() {
    let mut file = base_file();
    patch_header(&mut file, 0, |b| b[32] = 0b100);
    assert!(matches!(
        Container::open(Mem::new(file)),
        Err(ContainerError::UnsupportedFeatures { required: 0b100 })
    ));

    let mut file = base_file();
    patch_header(&mut file, 0, |b| b[40] = 1);
    let opened = Container::open(Mem::new(file.clone())).unwrap();
    assert_eq!(opened.optional_features(), 1);
    let mut w = Writer::update(Mem::new(file)).unwrap();
    w.add_section("more", kind(0x8000_0000), b"x", None)
        .unwrap();
    w.commit().unwrap();
    let opened = Container::open(w.into_sink()).unwrap();
    assert_eq!(opened.optional_features(), 1);
    assert_eq!(
        compact(&opened, Mem::default())
            .map(|s| Container::open(s).unwrap().optional_features())
            .unwrap(),
        1
    );

    let mut file = base_file();
    patch_header(&mut file, 0, |b| b[12] = 2);
    assert!(matches!(
        Container::open(Mem::new(file)),
        Err(ContainerError::UnsupportedVersion { major: 2, minor: 0 })
    ));

    let mut file = base_file();
    patch_header(&mut file, 0, |b| b[8..12].reverse());
    assert!(matches!(
        Container::open(Mem::new(file)),
        Err(ContainerError::UnsupportedByteOrder)
    ));
}

#[test]
fn a_section_with_an_unknown_codec_is_listed_verified_and_copied_but_not_read() {
    let mut file = base_file();
    // Entries are sorted by name; entry 4 is "x.unknown". Its codec is at byte 4.
    patch_toc(&mut file, |toc| toc[32 + 4 * 72 + 4] = 9);
    let opened = Container::open(Mem::new(file)).unwrap();
    let e = opened.get("x.unknown").unwrap();
    assert_eq!(e.codec, Codec::new(9));
    assert!(!e.codec.is_known());
    opened.verify("x.unknown").unwrap();
    assert!(matches!(
        opened.section("x.unknown"),
        Err(ContainerError::UnsupportedCodec { .. })
    ));
    let copy = Container::open(compact(&opened, Mem::default()).unwrap()).unwrap();
    assert_eq!(copy.get("x.unknown").unwrap().codec, Codec::new(9));
    copy.verify_all().unwrap();
}

#[test]
fn damage_is_detected_and_never_copied() {
    let file = base_file();
    let opened = Container::open(Mem::new(file.clone())).unwrap();
    let data = opened.get("adjacency.out.data").unwrap().clone();
    let at = |x: u64| usize::try_from(x).unwrap();

    let mut damaged = file.clone();
    damaged[at(data.offset + 100)] ^= 1;
    let c = Container::open(Mem::new(damaged)).unwrap();
    assert!(matches!(
        c.verify("adjacency.out.data"),
        Err(ContainerError::Damaged { .. })
    ));
    assert!(matches!(
        c.verify_all(),
        Err(ContainerError::Damaged { .. })
    ));
    assert!(matches!(
        compact(&c, Mem::default()),
        Err(ContainerError::Damaged { .. })
    ));

    // Non-zero padding is damage too.
    let mut damaged = file.clone();
    damaged[at(data.offset + data.len + 3)] = 1;
    let c = Container::open(Mem::new(damaged)).unwrap();
    assert!(matches!(
        c.verify("adjacency.out.data"),
        Err(ContainerError::Damaged { .. })
    ));

    // A damaged provenance record is refused, not misread.
    let record = opened.get("provenance/ingest").unwrap().clone();
    let mut damaged = file.clone();
    damaged[at(record.offset + 20)] ^= 0x40;
    let c = Container::open(Mem::new(damaged)).unwrap();
    assert!(matches!(
        c.provenance("adjacency.out.data"),
        Err(ContainerError::Damaged { .. })
    ));

    // A damaged table of contents with no older commit: the file cannot be opened.
    let mut damaged = file;
    let toc = at(opened.committed_len()) - 4096;
    damaged[toc + 40] ^= 1;
    assert!(matches!(
        Container::open(Mem::new(damaged)),
        Err(ContainerError::Corrupt(_))
    ));
}

#[test]
fn a_damaged_newest_table_falls_back_to_the_older_commit() {
    let mut w = Writer::update(Mem::new(base_file())).unwrap();
    w.add_section("late", kind(0x8000_0000), &bytes(8, 100), None)
        .unwrap();
    w.commit().unwrap();
    let mut file = w.into_sink().into_inner();
    let newest = Container::open(Mem::new(file.clone())).unwrap();
    let toc = usize::try_from(newest.committed_len()).unwrap() - 4096;
    file[toc + 40] ^= 1;
    let opened = Container::open(Mem::new(file.clone())).unwrap();
    assert_eq!(opened.generation(), 1);
    assert_eq!(opened.recovered_from(), Some(2));
    assert!(opened.get("late").is_none());
    opened.verify_all().unwrap();
    // Updating it continues from the older commit and overwrites the broken one.
    let mut w = Writer::update(Mem::new(file)).unwrap();
    w.add_section("again", kind(0x8000_0000), &bytes(9, 100), None)
        .unwrap();
    assert_eq!(w.commit().unwrap(), 2);
    let opened = Container::open(w.into_sink()).unwrap();
    assert_eq!(opened.recovered_from(), None);
    assert!(opened.get("again").is_some());
    opened.verify_all().unwrap();
}

// ---------------------------------------------------------------------------------
// The cost of opening.

/// A source that counts the reads made through it.
struct Counting<S> {
    inner: S,
    reads: Cell<usize>,
    bytes: Cell<u64>,
}

impl<S: ByteSource> ByteSource for Counting<S> {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        self.reads.set(self.reads.get() + 1);
        self.bytes.set(self.bytes.get() + buf.len() as u64);
        self.inner.read_at(offset, buf)
    }
}

#[test]
fn opening_costs_one_header_read_and_one_table_read() {
    let mut w = Writer::update(Mem::new(base_file())).unwrap();
    for i in 0..50 {
        w.add_section(
            &format!("results/{i}"),
            kind(0x8000_0000),
            &bytes(i, 64),
            None,
        )
        .unwrap();
    }
    w.commit().unwrap();
    let source = Counting {
        inner: w.into_sink(),
        reads: Cell::new(0),
        bytes: Cell::new(0),
    };
    let file = Container::open(&source).unwrap();
    assert_eq!(file.entries().len(), 55);
    assert_eq!(source.reads.get(), 2);
    // 4,224 bytes of header, and the table: 32 + 55 × 72 bytes plus the names.
    let names: u64 = file.entries().iter().map(|e| e.name.len() as u64).sum();
    assert_eq!(source.bytes.get(), 4096 + 128 + 32 + 55 * 72 + names);
}

// ---------------------------------------------------------------------------------
// Crash injection.

/// One operation on a sink.
#[derive(Clone, Debug)]
enum Op {
    Write(u64, Vec<u8>),
    SetLen(u64),
    Sync,
}

/// A sink that records every operation, and can be told to fail at one of them.
struct Recording {
    image: Mem,
    ops: Vec<Op>,
    fail_at: Option<usize>,
}

impl Recording {
    fn new(image: Vec<u8>) -> Self {
        Self {
            image: Mem::new(image),
            ops: Vec::new(),
            fail_at: None,
        }
    }

    fn record(&mut self, op: Op) -> Result<(), WriteError> {
        if self.fail_at == Some(self.ops.len()) {
            return Err(WriteError::Backend("injected fault".into()));
        }
        apply(&mut self.image, &op);
        self.ops.push(op);
        Ok(())
    }
}

impl ByteSource for Recording {
    fn len(&self) -> u64 {
        self.image.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        self.image.read_at(offset, buf)
    }
}

impl ByteSink for Recording {
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), WriteError> {
        self.record(Op::Write(offset, data.to_vec()))
    }

    fn set_len(&mut self, len: u64) -> Result<(), WriteError> {
        self.record(Op::SetLen(len))
    }

    fn sync(&mut self) -> Result<(), WriteError> {
        self.record(Op::Sync)
    }
}

fn apply(image: &mut Mem, op: &Op) {
    match op {
        Op::Write(at, data) => image.write_at(*at, data).unwrap(),
        Op::SetLen(len) => image.set_len(*len).unwrap(),
        Op::Sync => {}
    }
}

/// Whether `op` writes a header block (the commit point).
fn is_flip(op: &Op) -> bool {
    matches!(op, Op::Write(at, data) if *at < 8192 && data.len() == 128)
}

/// A small deterministic generator for the random crash images.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// How a pending (not yet synced) operation reached the disk in one crash image.
#[derive(Clone, Copy, Debug)]
enum Fate {
    Lost,
    Whole,
    /// Only the first `n` bytes of a write.
    Prefix(usize),
    /// Only the 512-byte sectors (counted from the start of the file) whose bit is set.
    Sectors(u64),
}

/// Applies `op` with `fate`; returns whether it reached the disk whole.
fn apply_with(image: &mut Mem, op: &Op, fate: Fate) -> bool {
    match (op, fate) {
        (_, Fate::Lost) => false,
        (_, Fate::Whole) => {
            apply(image, op);
            true
        }
        (Op::Write(at, data), Fate::Prefix(n)) => {
            image.write_at(*at, &data[..n.min(data.len())]).unwrap();
            n >= data.len()
        }
        (Op::Write(at, data), Fate::Sectors(mask)) => {
            let mut whole = true;
            let mut i = 0;
            while i < data.len() {
                let abs = *at + i as u64;
                let sector_end = (abs / 512 + 1) * 512;
                let n = usize::try_from(sector_end - abs)
                    .unwrap()
                    .min(data.len() - i);
                if mask >> ((abs / 512) % 64) & 1 == 1 {
                    image.write_at(abs, &data[i..i + n]).unwrap();
                } else {
                    whole = false;
                }
                i += n;
            }
            whole
        }
        (_, _) => {
            apply(image, op);
            true
        }
    }
}

/// Every crash image of `ops` applied to `start` that the sink contract allows when
/// the crash happens before operation `k`: what was synced is there; of what followed,
/// any part may be.
fn crash_images(start: &[u8], ops: &[Op], k: usize, rng: &mut Rng) -> Vec<(Mem, Vec<bool>)> {
    let synced = ops[..k]
        .iter()
        .rposition(|op| matches!(op, Op::Sync))
        .map_or(0, |s| s + 1);
    let pending = &ops[synced..k];
    let mut durable = Mem::new(start.to_vec());
    for op in &ops[..synced] {
        apply(&mut durable, op);
    }
    let mut fates: Vec<Vec<Fate>> = Vec::new();
    let n = pending.len();
    // The process was killed: everything written is there.
    fates.push(vec![Fate::Whole; n]);
    // Power was lost: nothing, each prefix in order, each operation alone, each
    // operation torn, random subsets with torn sectors.
    fates.push(vec![Fate::Lost; n]);
    for p in 1..n {
        fates.push(
            (0..n)
                .map(|i| if i < p { Fate::Whole } else { Fate::Lost })
                .collect(),
        );
    }
    for (i, op) in pending.iter().enumerate() {
        fates.push(
            (0..n)
                .map(|j| if j == i { Fate::Whole } else { Fate::Lost })
                .collect(),
        );
        if let Op::Write(_, data) = op {
            for cut in [1, data.len() / 2, data.len().saturating_sub(1)] {
                fates.push(
                    (0..n)
                        .map(|j| match j.cmp(&i) {
                            std::cmp::Ordering::Less => Fate::Whole,
                            std::cmp::Ordering::Equal => Fate::Prefix(cut),
                            std::cmp::Ordering::Greater => Fate::Lost,
                        })
                        .collect(),
                );
            }
        }
    }
    for _ in 0..12 {
        fates.push(
            (0..n)
                .map(|_| match rng.below(4) {
                    0 => Fate::Lost,
                    1 => Fate::Whole,
                    _ => Fate::Sectors(rng.next()),
                })
                .collect(),
        );
    }
    fates
        .into_iter()
        .map(|fate| {
            let mut image = durable.clone();
            let mut whole = vec![true; synced];
            for (op, f) in pending.iter().zip(fate) {
                whole.push(apply_with(&mut image, op, f));
            }
            (image, whole)
        })
        .collect()
}

/// What a file must read as: the sections of each generation, by name.
type States = BTreeMap<u64, BTreeMap<String, Vec<u8>>>;

/// Runs `script` against a recording sink over `start`, then checks every crash image
/// at every step: the file opens (or, for a file never committed, is refused as
/// such), reads as exactly the newest generation whose header block reached the disk
/// whole, every section verifies, and the file can be updated again.
fn check_crashes(start: Vec<u8>, script: impl Fn(&mut Writer<&mut Recording>) -> States) {
    let mut sink = Recording::new(start.clone());
    let mut states = States::new();
    let first_generation = Container::open(Mem::new(start.clone())).ok().map(|file| {
        states.insert(file.generation(), contents(&file));
        file.generation()
    });
    let committed = {
        let mut writer = if first_generation.is_some() {
            Writer::update(&mut sink).unwrap()
        } else {
            Writer::create(&mut sink).unwrap()
        };
        script(&mut writer)
    };
    let ops = sink.ops.clone();
    // The generation each header write commits, in order.
    let flips: Vec<(usize, u64)> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| is_flip(op))
        .map(|(i, _)| i)
        .zip(committed.keys().copied())
        .collect();
    assert_eq!(flips.len(), committed.len());
    states.extend(committed);
    let mut rng = Rng(0x5EED);
    let mut images = 0;
    for k in 0..=ops.len() {
        for (image, whole) in crash_images(&start, &ops, k, &mut rng) {
            images += 1;
            let expected = flips
                .iter()
                .filter(|(i, _)| whole.get(*i).copied().unwrap_or(false))
                .map(|&(_, g)| g)
                .max()
                .or(first_generation);
            let opened = Container::open(image);
            let Some(generation) = expected else {
                assert!(
                    matches!(
                        opened,
                        Err(ContainerError::NotAContainer | ContainerError::NotCommitted)
                    ),
                    "crash before op {k}: a file never committed must be refused as such, got {opened:?}"
                );
                continue;
            };
            let file = opened.unwrap_or_else(|e| panic!("crash before op {k}: {e}"));
            assert_eq!(file.generation(), generation, "crash before op {k}");
            if k == ops.len() {
                // Once `commit` has returned, its state survives any crash.
                assert_eq!(Some(generation), flips.last().map(|f| f.1));
            }
            assert_eq!(file.recovered_from(), None, "crash before op {k}");
            file.verify_all().unwrap();
            assert_eq!(contents(&file), states[&generation], "crash before op {k}");
            // The file can be written again, and the new commit holds everything.
            let mut w = Writer::update(file.into_source()).unwrap();
            w.add_section("after-crash", kind(0x8000_0009), &bytes(77, 33), None)
                .unwrap();
            w.commit().unwrap();
            let again = Container::open(w.into_sink()).unwrap();
            again.verify_all().unwrap();
            let mut want = states[&generation].clone();
            want.insert("after-crash".into(), bytes(77, 33));
            assert_eq!(contents(&again), want);
        }
    }
    assert!(images > ops.len());
}

#[test]
#[cfg_attr(miri, ignore = "safe code, thousands of hashes: hours under Miri")]
fn a_crash_at_any_step_of_creating_a_file_leaves_none_or_the_whole_file() {
    check_crashes(Vec::new(), |w| {
        w.add_provenance("p", &record("ingest")).unwrap();
        w.add_section("a", Kind::ADJACENCY_INDEX, &bytes(10, 700), Some("p"))
            .unwrap();
        w.add_section("b", Kind::ADJACENCY_DATA, &bytes(11, 5000), Some("p"))
            .unwrap();
        let generation = w.commit().unwrap();
        let file = Container::open(w.sink().image.clone()).unwrap();
        [(generation, contents(&file))].into_iter().collect()
    });
}

#[test]
#[cfg_attr(miri, ignore = "safe code, thousands of hashes: hours under Miri")]
fn a_crash_at_any_step_of_an_update_leaves_the_previous_or_the_new_state() {
    check_crashes(base_file(), |w| {
        w.add_provenance("provenance/pagerank", &record("pagerank"))
            .unwrap();
        w.add_section(
            "results/pagerank",
            kind(0x8000_0001),
            &bytes(12, 6000),
            Some("provenance/pagerank"),
        )
        .unwrap();
        w.remove("x.unknown").unwrap();
        w.remove("adjacency.out.index").unwrap();
        w.add_section(
            "adjacency.out.index",
            Kind::ADJACENCY_INDEX,
            &bytes(13, 400),
            Some("provenance/ingest"),
        )
        .unwrap();
        let generation = w.commit().unwrap();
        let file = Container::open(w.sink().image.clone()).unwrap();
        [(generation, contents(&file))].into_iter().collect()
    });
}

#[test]
#[cfg_attr(miri, ignore = "safe code, thousands of hashes: hours under Miri")]
fn a_crash_between_commits_of_one_writer_leaves_one_of_the_generations() {
    // The phases of `map` commit one by one (docs/ux-contract.md § 5.1).
    check_crashes(base_file(), |w| {
        let mut states = States::new();
        for (i, phase) in ["communities", "hierarchy", "layout"].iter().enumerate() {
            let record = format!("provenance/{phase}");
            w.add_provenance(&record, &self::record(phase)).unwrap();
            let seed = 20 + u64::try_from(i).unwrap();
            w.add_section(
                phase,
                kind(0x8000_0010),
                &bytes(seed, 3000 + 1000 * i),
                Some(&record),
            )
            .unwrap();
            let generation = w.commit().unwrap();
            let file = Container::open(w.sink().image.clone()).unwrap();
            states.insert(generation, contents(&file));
        }
        states
    });
}

#[test]
#[cfg_attr(miri, ignore = "safe code, thousands of hashes: hours under Miri")]
fn a_failed_write_poisons_the_writer_and_the_file_reads_as_before() {
    let start = base_file();
    let mut probe = Recording::new(start.clone());
    {
        let mut w = Writer::update(&mut probe).unwrap();
        w.add_section("new", kind(0x8000_0000), &bytes(30, 5000), None)
            .unwrap();
        w.commit().unwrap();
    }
    let steps = probe.ops.len();
    for fail_at in 0..steps {
        let mut sink = Recording::new(start.clone());
        sink.fail_at = Some(fail_at);
        let result = {
            let mut w = Writer::update(&mut sink).unwrap();
            let r = w
                .add_section("new", kind(0x8000_0000), &bytes(30, 5000), None)
                .and_then(|_| w.commit());
            if r.is_err() {
                assert!(matches!(w.commit(), Err(ContainerError::Poisoned)));
                assert!(matches!(
                    w.remove("ids.header"),
                    Err(ContainerError::Poisoned)
                ));
            }
            r
        };
        assert!(
            matches!(result, Err(ContainerError::Write(_))),
            "step {fail_at}"
        );
        let file = Container::open(sink.image).unwrap();
        // Every step before the header write fails cleanly; the header write itself
        // and the final sync have nothing after them that could still fail.
        let committed = fail_at > steps - 2;
        assert_eq!(
            file.generation(),
            if committed { 2 } else { 1 },
            "step {fail_at}"
        );
        file.verify_all().unwrap();
    }
}

// ---------------------------------------------------------------------------------
// Round trips.

/// An action of the round-trip test.
#[derive(Clone, Debug)]
enum Action {
    Add { name: u8, len: usize, seed: u64 },
    Remove { name: u8 },
    Commit,
    Reopen,
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        4 => (0u8..12, 0usize..6000, any::<u64>()).prop_map(|(name, len, seed)| Action::Add { name, len, seed }),
        2 => (0u8..12).prop_map(|name| Action::Remove { name }),
        1 => Just(Action::Commit),
        1 => Just(Action::Reopen),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    #[test]
    #[cfg_attr(miri, ignore = "safe code, thousands of hashes: hours under Miri")]
    fn random_appends_removals_and_reopens_round_trip(actions in prop::collection::vec(action(), 1..40)) {
        let mut committed: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut pending = committed.clone();
        let mut w = Writer::create(Mem::default()).unwrap();
        w.commit().unwrap();
        for a in actions {
            match a {
                Action::Add { name, len, seed } => {
                    let name = format!("s{name}");
                    let data = bytes(seed, len);
                    match w.add_section(&name, kind(0x8000_0000 + u32::try_from(name.len()).unwrap()), &data, None) {
                        Ok(_) => { pending.insert(name, data); }
                        Err(ContainerError::NameTaken(_)) => prop_assert!(pending.contains_key(&name)),
                        Err(e) => panic!("{e}"),
                    }
                }
                Action::Remove { name } => {
                    let name = format!("s{name}");
                    prop_assert_eq!(w.remove(&name).is_ok(), pending.remove(&name).is_some());
                }
                Action::Commit => {
                    w.commit().unwrap();
                    committed = pending.clone();
                }
                Action::Reopen => {
                    // Whatever was not committed is gone.
                    let file = Container::open(w.into_sink()).unwrap();
                    prop_assert_eq!(contents(&file), committed.clone());
                    file.verify_all().unwrap();
                    pending = committed.clone();
                    w = Writer::update(file.into_source()).unwrap();
                }
            }
        }
        w.commit().unwrap();
        let file = Container::open(w.into_sink()).unwrap();
        prop_assert_eq!(contents(&file), pending.clone());
        let compacted = Container::open(compact(&file, Mem::default()).unwrap()).unwrap();
        prop_assert_eq!(contents(&compacted), pending);
        prop_assert_eq!(compacted.reclaimable_bytes(), 0);
        let entries = |c: &Container<Mem>| c.entries().iter().map(|e| (e.name.clone(), e.hash)).collect::<Vec<_>>();
        prop_assert_eq!(entries(&compacted), entries(&file));
    }
}

#[test]
fn a_section_can_be_written_in_pieces() {
    let data = bytes(40, 70_000);
    let mut w = Writer::create(Mem::default()).unwrap();
    let mut section = w.begin_section("big", kind(0x8000_0000), None).unwrap();
    for piece in data.chunks(999) {
        section.write(piece).unwrap();
    }
    assert_eq!(section.len(), 70_000);
    let entry: Entry = section.finish().unwrap();
    assert_eq!(entry.hash, *blake3::hash(&data).as_bytes());
    // An abandoned section is not added and its bytes are reused.
    let abandoned = w.begin_section("gone", kind(0x8000_0000), None).unwrap();
    drop(abandoned);
    assert!(w.get("gone").is_none());
    w.commit().unwrap();
    let file = Container::open(w.into_sink()).unwrap();
    assert_eq!(contents(&file)["big"], data);
}
