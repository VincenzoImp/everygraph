// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The container on untrusted bytes (issue #191; `docs/research/sota-testing.md`,
//! challenge 1h): nothing may panic, and what is accepted must be consistent.
//!
//! The first byte picks the mode:
//! - 0: a table of contents, decoded at the offset the next 8 bytes give; one that
//!   decodes encodes and decodes again to the same entries;
//! - 1: a provenance record; one that decodes encodes and decodes again to itself;
//! - 2: a whole file: opened, verified (`verify_all` and a deep `verify`, which must
//!   agree), its records read, and compacted into a copy that verifies;
//! - 3: a decision seed: the first half of the input is a list of writer operations that
//!   build a valid file, the second half damages it. A deep `verify` that calls the
//!   damaged file intact must read every section as it was written; and the damaged
//!   file goes through mode 2, which reaches `verify_all` and `compact` on real files.

#![no_main]

use std::collections::BTreeMap;

use everygraph_core::{ByteSource, Memory, MemorySource};
use everygraph_format::adjacency::encode;
use everygraph_format::container::{Container, Kind, Writer, compact, fuzzing};
use everygraph_format::verify::{Depth, verify};
use libfuzzer_sys::fuzz_target;

type Mem = MemorySource<Vec<u8>>;

fn toc(bytes: &[u8]) {
    let Some((offset, toc)) = bytes.split_first_chunk::<8>() else {
        return;
    };
    let offset = u64::from_le_bytes(*offset);
    let Ok(entries) = fuzzing::decode_toc(toc, offset) else {
        return;
    };
    let again = fuzzing::encode_toc(&entries).expect("a decoded table encodes");
    assert_eq!(fuzzing::decode_toc(&again, offset).unwrap(), entries);
}

fn provenance(bytes: &[u8]) {
    let Ok(record) = fuzzing::decode_provenance(bytes) else {
        return;
    };
    let again = fuzzing::encode_provenance(&record).expect("a decoded record encodes");
    assert_eq!(fuzzing::decode_provenance(&again).unwrap(), record);
}

fn contents<S: ByteSource>(file: &Container<S>) -> BTreeMap<String, Vec<u8>> {
    file.entries()
        .iter()
        .filter_map(|e| {
            let section = file.section(&e.name).ok()?;
            let mut buf = vec![0; usize::try_from(e.len).ok()?];
            section.read_at(0, &mut buf).ok()?;
            Some((e.name.to_string(), buf))
        })
        .collect()
}

fn file(bytes: &[u8]) {
    let source = MemorySource::new(bytes);
    let report = verify(&source, Depth::Deep, &Memory::unlimited()).expect("memory does not fail");
    let Ok(file) = Container::open(&source) else {
        assert!(!report.is_intact());
        return;
    };
    let shallow = file.verify_all();
    if report.is_intact() {
        assert!(shallow.is_ok(), "verify finds nothing that verify_all finds: {shallow:?}");
    }
    for e in file.entries() {
        if e.kind == Kind::PROVENANCE {
            let _ = file.read_provenance(&e.name);
        }
    }
    if let Ok(copy) = compact(&file, Mem::default()) {
        let copy = Container::open(copy).expect("a compacted file opens");
        assert!(copy.verify_all().is_ok());
        assert_eq!(contents(&copy), contents(&file));
    }
}

/// Builds a file from `ops`, four bytes per operation.
fn build(ops: &[u8]) -> Vec<u8> {
    let mut w = Writer::create(Mem::default()).unwrap();
    for op in ops.chunks_exact(4) {
        let name = format!("s{}", op[1] % 8);
        let body: Vec<u8> = (0..usize::from(op[3]) * 37).map(|i| op[2] ^ i as u8).collect();
        let _ = match op[0] % 6 {
            0 => w.add_section(&name, Kind::PRIVATE, &body, None).map(drop),
            1 => {
                let record = everygraph_format::container::Provenance {
                    engine: name.clone(),
                    ..Default::default()
                };
                w.add_provenance(&format!("p{}", op[1] % 2), &record).map(drop)
            }
            2 => w.remove(&name).map(drop),
            3 => w.commit().map(drop),
            4 => {
                w.commit().unwrap();
                w = Writer::update(w.into_sink()).unwrap();
                Ok(())
            }
            _ => {
                // An adjacency of a few lists taken from the operation's bytes.
                let lists = [vec![1u32, 2], vec![u32::from(op[2] % 4)], vec![0, 1]];
                let offsets = [0u64, 2, 3, 5];
                let targets: Vec<u32> = lists.concat();
                match encode(&offsets, &targets) {
                    Ok(e) => w
                        .add_section(&format!("{name}.index"), Kind::ADJACENCY_INDEX, &e.index, None)
                        .and_then(|_| {
                            w.add_section(&format!("{name}.data"), Kind::ADJACENCY_DATA, &e.data, None)
                        })
                        .map(drop),
                    Err(_) => Ok(()),
                }
            }
        };
    }
    w.commit().unwrap();
    w.into_sink().into_inner()
}

fn built_then_damaged(bytes: &[u8]) {
    let (ops, damage) = bytes.split_at(bytes.len() / 2);
    let clean = build(ops);
    let (generation, written) = {
        let file = Container::open(MemorySource::new(clean.as_slice())).unwrap();
        (file.generation(), contents(&file))
    };
    assert!(
        verify(&MemorySource::new(clean.as_slice()), Depth::Deep, &Memory::unlimited()).unwrap().notes.is_empty(),
        "a file written without failures has nothing to note"
    );
    let mut damaged = clean;
    for hit in damage.chunks_exact(3) {
        let at = usize::from(u16::from_le_bytes([hit[0], hit[1]])) * 7 % damaged.len();
        damaged[at] ^= hit[2];
    }
    file(&damaged);
    let source = MemorySource::new(damaged.as_slice());
    let report = verify(&source, Depth::Deep, &Memory::unlimited()).unwrap();
    if report.is_intact() {
        let file = Container::open(&source).unwrap();
        if file.generation() == generation {
            assert_eq!(contents(&file), written, "damage that verify did not see");
        } else {
            // A damaged newest table or header block reads like a commit cut short: the
            // older commit is whole, and verify names the lost one or notes the block.
            assert!(
                report.lost_generation.is_some() || !report.notes.is_empty(),
                "a commit lost without a word"
            );
        }
    }
}

fuzz_target!(|bytes: &[u8]| {
    let Some((&mode, rest)) = bytes.split_first() else {
        return;
    };
    match mode % 4 {
        0 => toc(rest),
        1 => provenance(rest),
        2 => file(rest),
        _ => built_then_damaged(rest),
    }
});
