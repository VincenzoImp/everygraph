// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The column reader on untrusted bytes (`docs/spec/eg-columns.md` § 7): parsing a
//! layout, checking a column section against it, reading dictionary strings and parsing
//! statistics must never panic, read out of bounds or allocate beyond the bytes given;
//! a layout that parses places every buffer inside its column.
//!
//! The first byte picks the mode:
//! - even: the rest is raw input: a little-endian `u16` length of the layout, the
//!   layout, a `u16` length of the statistics, the statistics, and the column;
//! - odd: the rest describes a column (type, rows, batch rows, nulls, values), which is
//!   written, read back (it must round-trip), then damaged as the input's tail says and
//!   read again. This reaches framing that is valid up to a late byte, which raw input
//!   rarely does.

#![no_main]

use everygraph_core::{ByteSource, MemorySource};
use everygraph_format::columns::{
    Bits, CodeWidth, Column, ColumnLayout, ColumnSpec, ColumnStats, ColumnType, ColumnWriter,
    Dictionary, SectionNames, Values,
};
use everygraph_format::container::{Container, Writer};
use libfuzzer_sys::fuzz_target;

type Mem = MemorySource<Vec<u8>>;

fn check(layout: &[u8], stats: &[u8], column: &[u8]) -> bool {
    let Ok(layout) = ColumnLayout::parse(layout) else {
        return false;
    };
    let len = layout.column_len();
    for b in layout.batches() {
        assert!(b.validity.end <= len && b.values.end <= len);
        assert!(b.validity.start <= b.validity.end && b.values.start < b.values.end);
    }
    if let Some(d) = layout.dictionary() {
        assert!(d.offsets.end <= len && d.data.end <= len);
        assert!(d.offsets.start <= d.offsets.end && d.data.start <= d.data.end);
    }
    let _ = ColumnStats::parse(stats, &layout);
    let Ok(column) = Column::open(&layout, Mem::new(column.to_vec())) else {
        return false;
    };
    if let Some(d) = layout.dictionary() {
        let mut buf = Vec::new();
        for code in 0..d.values.min(300) + 1 {
            let _ = column.dictionary_entry(code, &mut buf);
        }
    }
    true
}

const TYPES: [ColumnType; 6] = [
    ColumnType::Float64,
    ColumnType::Int32,
    ColumnType::Bool,
    ColumnType::Dictionary(CodeWidth::U8),
    ColumnType::Decimal256 {
        precision: 76,
        scale: 2,
    },
    ColumnType::FixedSizeBinary(3),
];

fn section(file: &Container<Mem>, name: &str) -> Vec<u8> {
    let s = file.section(name).unwrap();
    let mut buf = vec![0; usize::try_from(s.len()).unwrap()];
    s.read_at(0, &mut buf).unwrap();
    buf
}

fn structured(bytes: &[u8]) {
    let [t, rows, r, nulls, rest @ ..] = bytes else {
        return;
    };
    let ty = TYPES[usize::from(*t) % TYPES.len()];
    let rows = usize::from(*rows);
    let batch_rows = 1usize << (r % 6);
    let width = usize::try_from(ty.width().max(1)).unwrap();
    let (values, damage) = rest.split_at(rest.len().min(rows * width * 2));
    let mut raw = values.to_vec();
    raw.resize(rows * width.max(8), 0);
    let (offsets, bytes) = Dictionary::parts(&["a", "", "ccc"]);
    let f64s: Vec<f64> = raw
        .chunks(8)
        .take(rows)
        .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let i32s: Vec<i32> = raw
        .chunks(8)
        .take(rows)
        .map(|c| i32::from_le_bytes(c[..4].try_into().unwrap()))
        .collect();
    let bools: Vec<bool> = raw.iter().take(rows).map(|b| b & 1 == 1).collect();
    let codes: Vec<u8> = raw.iter().take(rows).map(|b| b % 3).collect();
    // Decimal256 values within the precision: small integers, sign-extended.
    let wide: Vec<u8> = raw
        .chunks(8)
        .take(rows)
        .flat_map(|c| {
            i128::from(i64::from_le_bytes(c.try_into().unwrap()))
                .to_le_bytes()
                .into_iter()
                .chain([if c[7] & 0x80 == 0 { 0 } else { 0xFF }; 16])
        })
        .collect();
    let all: Values<'_> = match ty {
        ColumnType::Float64 => Values::F64(&f64s),
        ColumnType::Int32 => Values::I32(&i32s),
        ColumnType::Bool => Values::Bool(&bools),
        ColumnType::Dictionary(_) => Values::U8(&codes),
        ColumnType::Decimal256 { .. } => Values::Bytes(&wide),
        _ => Values::Bytes(&raw[..rows * 3]),
    };
    let spec = ColumnSpec {
        nullable: *nulls != 0,
        batch_rows: batch_rows as u64,
        dictionary: matches!(ty, ColumnType::Dictionary(_)).then_some(Dictionary {
            offsets: &offsets,
            bytes: &bytes,
        }),
        ..ColumnSpec::new("c", ty, rows as u64)
    };
    let mut w = Writer::create(Mem::new(Vec::new())).unwrap();
    let mut c = ColumnWriter::begin(&mut w, "t", &spec, None).unwrap();
    for b in 0..rows.div_ceil(batch_rows) {
        let range = b * batch_rows..rows.min(b * batch_rows + batch_rows);
        let n = range.len();
        let per = match ty {
            ColumnType::Decimal256 { .. } => 32,
            ColumnType::FixedSizeBinary(_) => 3,
            _ => 1,
        };
        let part = match all {
            Values::F64(v) => Values::F64(&v[range.clone()]),
            Values::I32(v) => Values::I32(&v[range.clone()]),
            Values::Bool(v) => Values::Bool(&v[range.clone()]),
            Values::U8(v) => Values::U8(&v[range.clone()]),
            Values::Bytes(v) => Values::Bytes(&v[range.start * per..range.end * per]),
            _ => unreachable!(),
        };
        // The batch's validity as two pieces of one bitmap, cut at a bit offset.
        let bitmap: Vec<u8> = (0..n.div_ceil(8) + 1)
            .map(|i| !(nulls.rotate_left(i as u32)))
            .collect();
        let cut = n / 2;
        let pieces = [
            Bits {
                bytes: &bitmap,
                offset: 3,
                len: cut as u64,
            },
            Bits {
                bytes: &bitmap,
                offset: 3 + cut as u64,
                len: (n - cut) as u64,
            },
        ];
        let validity: &[Bits<'_>] = if spec.nullable { &pieces } else { &[] };
        c.write_batch(&[part], validity).unwrap();
    }
    c.finish().unwrap();
    w.commit().unwrap();
    let file = Container::open(w.into_sink()).unwrap();
    let names = SectionNames::new("t", "c");
    let mut parts = [
        section(&file, &names.layout),
        file.get(&names.stats)
            .map(|_| section(&file, &names.stats))
            .unwrap_or_default(),
        section(&file, &names.column),
    ];
    assert!(
        check(&parts[0], &parts[1], &parts[2]),
        "a column the writer wrote is refused"
    );
    for hit in damage.chunks_exact(3) {
        let part = &mut parts[usize::from(hit[0]) % 3];
        if !part.is_empty() {
            let i = usize::from(u16::from_le_bytes([hit[1], hit[0]])) * 31 % part.len();
            part[i] ^= hit[2] | 1;
        }
    }
    check(&parts[0], &parts[1], &parts[2]);
}

fuzz_target!(|bytes: &[u8]| {
    let Some((&mode, rest)) = bytes.split_first() else {
        return;
    };
    if mode & 1 == 1 {
        structured(rest);
        return;
    }
    let take = |b: &[u8]| -> Option<(Vec<u8>, Vec<u8>)> {
        let (len, b) = b.split_first_chunk::<2>()?;
        let n = usize::from(u16::from_le_bytes(*len)).min(b.len());
        Some((b[..n].to_vec(), b[n..].to_vec()))
    };
    let Some((layout, rest)) = take(rest) else {
        return;
    };
    let Some((stats, column)) = take(&rest) else {
        return;
    };
    check(&layout, &stats, &column);
});
