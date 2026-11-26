// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `verify --deep` on property columns (`docs/spec/eg-columns.md` § 6, § 7): sound
//! columns are intact; a column without its layout, statistics that are not those of
//! the values, a stored value that is not canonical, and rows that do not fit the
//! adjacency's scope are each found, behind valid section hashes.

use everygraph_core::{ByteSource, Memory, MemorySource};
use everygraph_format::adjacency::encode;
use everygraph_format::columns::{ColumnSpec, ColumnType, SectionNames, write_column};
use everygraph_format::container::{Container, Kind, Writer};
use everygraph_format::verify::{Depth, verify};

type Mem = MemorySource<Vec<u8>>;

/// The sections of a file: name, kind, bytes.
type Sections = Vec<(String, Kind, Vec<u8>)>;

/// A file with the directed adjacency 0→1, 0→2, 1→2 (3 entries) and the pair column
/// `w` of `rows` rows.
fn file(rows: u64) -> Sections {
    let mut w = Writer::create(Mem::default()).unwrap();
    let e = encode(&[0, 2, 3, 3], &[1, 2, 2]).unwrap();
    w.add_section("adjacency.out.index", Kind::ADJACENCY_INDEX, &e.index, None)
        .unwrap();
    w.add_section("adjacency.out.data", Kind::ADJACENCY_DATA, &e.data, None)
        .unwrap();
    let values: Vec<f64> = (0..rows).map(|i| i as f64 / 2.0).collect();
    let spec = ColumnSpec::new("w", ColumnType::Float64, rows);
    write_column(&mut w, "adjacency.out.pair", &spec, &values[..], None).unwrap();
    w.commit().unwrap();
    let file = Container::open(w.into_sink()).unwrap();
    file.entries()
        .iter()
        .map(|e| {
            let section = file.section(&e.name).unwrap();
            let mut bytes = vec![0; usize::try_from(section.len()).unwrap()];
            section.read_at(0, &mut bytes).unwrap();
            (e.name.to_string(), e.kind, bytes)
        })
        .collect()
}

fn rebuild(sections: &Sections) -> Vec<u8> {
    let mut w = Writer::create(Mem::default()).unwrap();
    for (name, kind, bytes) in sections {
        w.add_section(name, *kind, bytes, None).unwrap();
    }
    w.commit().unwrap();
    w.into_sink().into_inner()
}

fn problems(sections: &Sections) -> Vec<String> {
    let bytes = rebuild(sections);
    let report = verify(
        &MemorySource::new(&bytes[..]),
        Depth::Deep,
        &Memory::unlimited(),
    )
    .unwrap();
    report.problems.into_iter().map(|p| p.what).collect()
}

fn names() -> SectionNames {
    SectionNames::new("adjacency.out.pair", "w")
}

#[test]
fn a_sound_column_is_intact() {
    assert_eq!(problems(&file(3)), Vec::<String>::new());
}

#[test]
fn a_column_must_have_the_rows_of_its_scope() {
    let found = problems(&file(4));
    assert!(found.iter().any(|p| p.contains("4 rows")), "{found:?}");
}

#[test]
fn a_column_needs_its_layout_and_a_numeric_one_its_statistics() {
    for missing in [names().layout, names().stats] {
        let mut sections = file(3);
        sections.retain(|s| s.0 != missing);
        let found = problems(&sections);
        assert_eq!(found.len(), 1, "{missing}: {found:?}");
    }
    let mut sections = file(3);
    sections.retain(|s| s.0 != names().column);
    let found = problems(&sections);
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn statistics_must_be_those_of_the_values() {
    let mut sections = file(3);
    let at = sections.iter().position(|s| s.0 == names().stats).unwrap();
    // A valid record, with another sum.
    let sum = f64::from_le_bytes(sections[at].2[40..48].try_into().unwrap());
    sections[at].2[40..48].copy_from_slice(&(sum + 1.0).to_bits().to_le_bytes());
    let found = problems(&sections);
    assert!(
        found.iter().any(|p| p.contains("not those of its values")),
        "{found:?}"
    );
}

#[test]
fn a_value_that_is_not_canonical_is_found() {
    let mut sections = file(3);
    let column = sections.iter().position(|s| s.0 == names().column).unwrap();
    // The values 0.0, 0.5, 1.0 start a page of the section; make the first −0.0.
    let bytes = &mut sections[column].2;
    let page = (4096..bytes.len())
        .step_by(4096)
        .find(|&p| bytes[p + 8..p + 16] == 0.5f64.to_le_bytes())
        .unwrap();
    bytes[page..page + 8].copy_from_slice(&(-0.0f64).to_le_bytes());
    let found = problems(&sections);
    assert!(
        found.iter().any(|p| p.contains("not canonical")),
        "{found:?}"
    );
}
