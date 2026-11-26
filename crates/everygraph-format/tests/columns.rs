// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests of property columns (`docs/spec/eg-columns.md`): golden bytes, round trips of
//! every type against values and statistics computed here independently, the exact
//! sum against exact integer arithmetic, and damage: every byte of a column section
//! outside its buffers is checked by the reader.
//!
//! What an Arrow reader makes of the same bytes is checked by
//! `tools/check_columns_python.py` (pyarrow and Polars).

#![forbid(unsafe_code)]
#![cfg(not(miri))]

use everygraph_core::{ByteSource, MemorySource};
use everygraph_format::columns::{
    Bits, CodeWidth, Column, ColumnError, ColumnLayout, ColumnSpec, ColumnStats, ColumnType,
    ColumnWriter, Dictionary, SectionNames, TimeUnit, Values, write_column,
};
use everygraph_format::container::{Container, ContainerError, Writer};
use everygraph_oracle::rng::SplitMix64;
use proptest::prelude::*;

type Mem = MemorySource<Vec<u8>>;

fn section(file: &Container<Mem>, name: &str) -> Option<Vec<u8>> {
    let s = file.section(name).ok()?;
    let mut buf = vec![0; usize::try_from(s.len()).unwrap()];
    s.read_at(0, &mut buf).unwrap();
    Some(buf)
}

/// The three sections of a column: Arrow file, layout, statistics.
fn sections(file: &Container<Mem>, scope: &str, name: &str) -> (Vec<u8>, Vec<u8>, Option<Vec<u8>>) {
    let n = SectionNames::new(scope, name);
    (
        section(file, &n.column).unwrap(),
        section(file, &n.layout).unwrap(),
        section(file, &n.stats),
    )
}

fn commit(w: Writer<Mem>) -> Container<Mem> {
    let mut w = w;
    w.commit().unwrap();
    Container::open(w.into_sink()).unwrap()
}

/// Each row's stored bytes as the layout places them (`None` when null): the value's
/// `width` bytes, or one byte 0/1 for `bool`.
fn stored_rows(layout: &ColumnLayout, column: &[u8]) -> Vec<Option<Vec<u8>>> {
    let at = |x: u64| usize::try_from(x).unwrap();
    let width = at(layout.ty().width());
    let mut out = Vec::new();
    for (i, b) in (0u64..).zip(layout.batches()) {
        let rows = layout
            .batch_rows()
            .min(layout.rows() - i * layout.batch_rows());
        let bit = |start: u64, r: u64| column[at(start + r / 8)] >> (r % 8) & 1 == 1;
        for r in 0..rows {
            if b.null_count > 0 && !bit(b.validity.start, r) {
                out.push(None);
            } else if width == 0 {
                out.push(Some(vec![u8::from(bit(b.values.start, r))]));
            } else {
                let s = at(b.values.start) + at(r) * width;
                out.push(Some(column[s..s + width].to_vec()));
            }
        }
    }
    out
}

/// Writes one column of `scope/name` and reads it back through the public reader.
fn round_trip(spec: &ColumnSpec<'_>, batches: &[(Vec<Values<'_>>, Vec<Bits<'_>>)]) -> Read {
    let mut w = Writer::create(Mem::new(Vec::new())).unwrap();
    let mut c = ColumnWriter::begin(&mut w, "t", spec, None).unwrap();
    for (values, validity) in batches {
        c.write_batch(values, validity).unwrap();
    }
    let returned = c.finish().unwrap();
    let file = commit(w);
    let (column, layout_bytes, stats_bytes) = sections(&file, "t", spec.name);
    let layout = ColumnLayout::parse(&layout_bytes).unwrap();
    let opened = Column::open(&layout, Mem::new(column.clone()));
    assert!(opened.is_ok(), "{opened:?}");
    let stats = stats_bytes.map(|b| ColumnStats::parse(&b, &layout).unwrap());
    assert_eq!(stats.as_ref().map(key), returned.as_ref().map(key));
    Read {
        rows: stored_rows(&layout, &column),
        layout,
        layout_bytes,
        column,
        stats,
    }
}

/// Statistics with the sum as bits, so that a NaN sum compares equal to itself.
type Key = (u64, u64, u64, u64, Option<([u8; 32], [u8; 32])>);

fn key(s: &ColumnStats) -> Key {
    (s.count, s.null_count, s.nan_count, s.sum.to_bits(), s.range)
}

struct Read {
    layout: ColumnLayout,
    layout_bytes: Vec<u8>,
    column: Vec<u8>,
    rows: Vec<Option<Vec<u8>>>,
    stats: Option<ColumnStats>,
}

// ---------------------------------------------------------------------------------------
// Golden bytes.

/// BLAKE3 of the column, layout and statistics sections of three small columns. A
/// change here changes the bytes of every graph file: it needs a new version of the
/// specification (§ 8), and the pyarrow check must pass on the new bytes.
#[test]
fn golden_bytes() {
    let mut w = Writer::create(Mem::new(Vec::new())).unwrap();
    let spec = ColumnSpec::new("x", ColumnType::Int32, 3);
    write_column(&mut w, "nodes", &spec, &[1i32, -2, 3], None).unwrap();

    let spec = ColumnSpec {
        nullable: true,
        batch_rows: 2,
        ..ColumnSpec::new("weight", ColumnType::Float64, 3)
    };
    let mut c = ColumnWriter::begin(&mut w, "adjacency.out.pair", &spec, None).unwrap();
    let bits = Bits {
        bytes: &[0b01],
        offset: 0,
        len: 2,
    };
    c.write_batch(&[Values::F64(&[0.5, -0.0])], &[bits])
        .unwrap();
    c.write_batch(&[Values::F64(&[f64::NAN])], &[]).unwrap();
    c.finish().unwrap();

    let (offsets, bytes) = Dictionary::parts(&["swap", "mint"]);
    let spec = ColumnSpec {
        dictionary: Some(Dictionary {
            offsets: &offsets,
            bytes: &bytes,
        }),
        ..ColumnSpec::new("kind", ColumnType::Dictionary(CodeWidth::U16), 4)
    };
    write_column(&mut w, "edges", &spec, &[1u16, 0, 0, 1], None).unwrap();
    let file = commit(w);

    let hash = |b: &[u8]| blake3::hash(b).to_hex().to_string();
    let got: Vec<(String, String)> = file
        .entries()
        .iter()
        .map(|e| (e.name.to_string(), hash(&section(&file, &e.name).unwrap())))
        .collect();
    let want = [
        (
            "adjacency.out.pair/arrow/weight",
            "bca48b0ab4c08356945f0282ce30c692f8003ea534ec388895c25e6cd347288b",
        ),
        (
            "adjacency.out.pair/layout/weight",
            "d9311102373b6f0632e9d4e08da33e702e44a2c6e168707392652dd8baf9728d",
        ),
        (
            "adjacency.out.pair/stats/weight",
            "d794af388a3e86ca30c6888f0249206fc78ddbd781909051a27e9feeb54adbae",
        ),
        (
            "edges/arrow/kind",
            "1226ca6333d914f7175e5659629fba81fa5e20682587d3be3060771af04f4a3a",
        ),
        (
            "edges/layout/kind",
            "f03f63d7a6e863b86b511db1dd1332f4334053a99cbb9f047fe3c9aa3fbf7e64",
        ),
        (
            "nodes/arrow/x",
            "5c8ba0cc99315ade04470d149056abf7b7f2b2dff06b9d6d279711e5cbc5c4e2",
        ),
        (
            "nodes/layout/x",
            "913047af50e50c070b17c4ea656c93e00f70862a429da6067e5a9a092f4a645b",
        ),
        (
            "nodes/stats/x",
            "245a5c5e58bdd61900f0bf377c6b390d958a3104f566de854389d64c71f930d1",
        ),
    ];
    let want: Vec<(String, String)> = want
        .iter()
        .map(|(a, b)| ((*a).into(), (*b).into()))
        .collect();
    assert_eq!(got, want);

    // The layout and statistics of `nodes/x` in full (§ 4, § 5).
    let (column, layout, stats) = sections(&file, "nodes", "x");
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    assert_eq!(
        hex(&layout),
        concat!(
            "4547434f4c4c4159",
            "01000000",
            "00000000", // magic, version, flags
            "0300000000000000",
            "0000000400000000", // rows 3, batch rows 2^26
            "0100000000000000",
            "ca10000000000000", // 1 batch, column of 4,298 bytes
            "01",
            "00",
            "0000",
            "20000000",
            "01000000", // int32: int, not nullable, 32 bits, signed
            "01000000",
            "0000000000000000",
            "0000000000000000", // name length, no dictionary
            "7800000000000000", // "x", zero bytes to 8
            "0000000000000000", // batch 0: no null
            "0010000000000000",
            "0000000000000000", // validity: at 4096, empty
            "0010000000000000",
            "0c00000000000000", // values: at 4096, 12 bytes
        )
    );
    let zeros = |n| "00".repeat(n);
    assert_eq!(
        hex(&stats.unwrap()),
        [
            "4547434f4c535441",
            "01000000",
            "01000000", // magic, version, min and max present
            "0300000000000000",
            "0000000000000000",
            "0000000000000000", // 3 values, no null, no NaN
            "0000000000000040", // sum 2.0
            "feffffff",
            &zeros(28), // min −2
            "03000000",
            &zeros(28), // max 3
        ]
        .concat()
    );
    // The Arrow file starts with its magic and the schema message (§ 3.2), and its
    // only batch's values are at 4096.
    assert_eq!(hex(&column[..8]), "4152524f57310000");
    let layout = ColumnLayout::parse(&layout).unwrap();
    assert_eq!(layout.batches()[0].values, 4096..4108);
    assert_eq!(
        &column[4096..4108],
        &[1, 0, 0, 0, 0xFE, 0xFF, 0xFF, 0xFF, 3, 0, 0, 0]
    );
}

// ---------------------------------------------------------------------------------------
// Round trips of every type, against values and statistics computed here.

const TYPES: &[ColumnType] = &[
    ColumnType::Int8,
    ColumnType::Int16,
    ColumnType::Int32,
    ColumnType::Int64,
    ColumnType::UInt8,
    ColumnType::UInt16,
    ColumnType::UInt32,
    ColumnType::UInt64,
    ColumnType::Float32,
    ColumnType::Float64,
    ColumnType::Bool,
    ColumnType::Timestamp(TimeUnit::Nanosecond),
    ColumnType::FixedSizeBinary(5),
    ColumnType::Decimal128 {
        precision: 30,
        scale: 4,
    },
    ColumnType::Decimal128 {
        precision: 30,
        scale: -3,
    },
    ColumnType::Decimal256 {
        precision: 76,
        scale: 18,
    },
    ColumnType::Dictionary(CodeWidth::U8),
    ColumnType::Dictionary(CodeWidth::U16),
    ColumnType::Dictionary(CodeWidth::U32),
];

/// Values of one Rust type, as the writer takes them.
enum Owned {
    I8(Vec<i8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
    U64(Vec<u64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    Bool(Vec<bool>),
    I128(Vec<i128>),
    Bytes(Vec<u8>, usize),
}

impl Owned {
    fn rows(&self, r: std::ops::Range<usize>) -> Values<'_> {
        match self {
            Self::I8(v) => Values::I8(&v[r]),
            Self::I16(v) => Values::I16(&v[r]),
            Self::I32(v) => Values::I32(&v[r]),
            Self::I64(v) => Values::I64(&v[r]),
            Self::U8(v) => Values::U8(&v[r]),
            Self::U16(v) => Values::U16(&v[r]),
            Self::U32(v) => Values::U32(&v[r]),
            Self::U64(v) => Values::U64(&v[r]),
            Self::F32(v) => Values::F32(&v[r]),
            Self::F64(v) => Values::F64(&v[r]),
            Self::Bool(v) => Values::Bool(&v[r]),
            Self::I128(v) => Values::I128(&v[r]),
            Self::Bytes(v, w) => Values::Bytes(&v[r.start * w..r.end * w]),
        }
    }
}

/// The number a stored value stands for, for the statistics.
#[derive(Clone, Copy)]
enum Number {
    Int(i128),
    /// A float that is exactly `units × 2^−20` (0 for NaN and infinities).
    Float(f64, i128),
    None,
}

/// A generated column: the values given to the writer, which rows are valid, the bytes
/// each row must be stored as, and what each stands for.
struct Case {
    values: Owned,
    valid: Vec<bool>,
    stored: Vec<Vec<u8>>,
    numbers: Vec<Number>,
    strings: Vec<String>,
}

/// Bytes, numbers and values of integers of one type.
macro_rules! ints {
    ($raw:expr, $t:ty, $variant:ident) => {{
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let v: Vec<$t> = $raw.iter().map(|&r| r as $t).collect();
        let s = v.iter().map(|x| x.to_le_bytes().to_vec()).collect();
        let n = v.iter().map(|&x| Number::Int(x.into())).collect();
        (Owned::$variant(v), s, n)
    }};
}

/// A float `m × 2^e`, `m` of `bits` bits and `e` in −20..=20, or (1 time in 32 each)
/// a NaN with a payload, −0.0, an infinity; with its value in units of 2^−20.
fn float(r: u64, bits: u32) -> (f64, i128) {
    match r % 32 {
        0 => (f64::from_bits(0x7FF0_0000_0000_0000 | (r >> 12 | 1)), 0),
        1 => (-0.0, 0),
        2 if r & 64 == 0 => (f64::INFINITY, 0),
        2 => (f64::NEG_INFINITY, 0),
        _ => {
            #[allow(clippy::cast_possible_wrap)]
            let m = (r as i64) >> (64 - bits);
            let e = i32::try_from((r >> 8) % 41).unwrap() - 20;
            #[allow(clippy::cast_precision_loss)]
            let x = m as f64 * pow2(e);
            (x, i128::from(m) << (e + 20))
        }
    }
}

/// 2^e, exactly, for a normal power of two.
fn pow2(e: i32) -> f64 {
    f64::from_bits(u64::try_from(e + 1023).unwrap() << 52)
}

fn canonical64(x: f64) -> f64 {
    if x.is_nan() {
        f64::from_bits(0x7FF8_0000_0000_0000)
    } else if x == 0.0 {
        0.0
    } else {
        x
    }
}

#[allow(clippy::cast_possible_truncation)]
fn generate(ty: ColumnType, rows: usize, seed: u64, nullable: bool) -> Case {
    let mut rng = SplitMix64::new(seed);
    let valid: Vec<bool> = (0..rows)
        .map(|_| !nullable || !rng.next_u64().is_multiple_of(4))
        .collect();
    let raw: Vec<u64> = (0..rows).map(|_| rng.next_u64()).collect();
    let strings: Vec<String> = (0..1 + seed % 7)
        .map(|i| {
            "αβc"
                .chars()
                .cycle()
                .take(usize::try_from((seed >> 8) % 5 + i).unwrap())
                .collect()
        })
        .collect();
    let k = strings.len() as u64;
    let none = vec![Number::None; rows];
    let (values, stored, numbers): (Owned, Vec<Vec<u8>>, Vec<Number>) = match ty {
        ColumnType::Int8 => ints!(raw, i8, I8),
        ColumnType::Int16 => ints!(raw, i16, I16),
        ColumnType::Int32 => ints!(raw, i32, I32),
        ColumnType::Int64 | ColumnType::Timestamp(_) => ints!(raw, i64, I64),
        ColumnType::UInt8 => ints!(raw, u8, U8),
        ColumnType::UInt16 => ints!(raw, u16, U16),
        ColumnType::UInt32 => ints!(raw, u32, U32),
        ColumnType::UInt64 => ints!(raw, u64, U64),
        ColumnType::Float32 => {
            let f: Vec<(f64, i128)> = raw.iter().map(|&r| float(r, 24)).collect();
            let v: Vec<f32> = f.iter().map(|&(x, _)| x as f32).collect();
            let s = f
                .iter()
                .map(|&(x, _)| {
                    let x = canonical64(x);
                    let bits = if x.is_nan() {
                        0x7FC0_0000
                    } else {
                        (x as f32).to_bits()
                    };
                    bits.to_le_bytes().to_vec()
                })
                .collect();
            let n = f.iter().map(|&(x, u)| Number::Float(x, u)).collect();
            (Owned::F32(v), s, n)
        }
        ColumnType::Float64 => {
            let f: Vec<(f64, i128)> = raw.iter().map(|&r| float(r, 40)).collect();
            let v: Vec<f64> = f.iter().map(|&(x, _)| x).collect();
            let s = v
                .iter()
                .map(|&x| canonical64(x).to_le_bytes().to_vec())
                .collect();
            let n = f.iter().map(|&(x, u)| Number::Float(x, u)).collect();
            (Owned::F64(v), s, n)
        }
        ColumnType::Bool => {
            let v: Vec<bool> = raw.iter().map(|&r| r & 1 == 1).collect();
            let s = v.iter().map(|&x| vec![u8::from(x)]).collect();
            (Owned::Bool(v), s, none)
        }
        ColumnType::FixedSizeBinary(w) => {
            let w = usize::try_from(w).unwrap();
            let v: Vec<u8> = raw
                .iter()
                .flat_map(|r| r.to_le_bytes()[..w].to_vec())
                .collect();
            let s = v.chunks(w).map(<[u8]>::to_vec).collect();
            (Owned::Bytes(v, w), s, none)
        }
        ColumnType::Decimal128 { .. } | ColumnType::Decimal256 { .. } => {
            let limit = 10i128.pow(30);
            let sign = |r: u64| if r & 1 == 0 { 1 } else { -1 };
            let v: Vec<i128> = raw
                .iter()
                .map(|&r| (i128::from(r) * i128::from(r >> 3) * sign(r)) % limit)
                .collect();
            let n = v.iter().map(|&x| Number::Int(x)).collect();
            if matches!(ty, ColumnType::Decimal128 { .. }) {
                let s = v.iter().map(|x| x.to_le_bytes().to_vec()).collect();
                (Owned::I128(v), s, n)
            } else {
                let s: Vec<Vec<u8>> = v
                    .iter()
                    .map(|&x| [x.to_le_bytes(), [if x < 0 { 0xFF } else { 0 }; 16]].concat())
                    .collect();
                (Owned::Bytes(s.concat(), 32), s, n)
            }
        }
        ColumnType::Dictionary(width) => {
            let codes: Vec<u64> = raw.iter().map(|&r| r % k).collect();
            let (v, s, _): (Owned, Vec<Vec<u8>>, Vec<Number>) = match width {
                CodeWidth::U8 => ints!(codes, u8, U8),
                CodeWidth::U16 => ints!(codes, u16, U16),
                CodeWidth::U32 => ints!(codes, u32, U32),
            };
            (v, s, none)
        }
        _ => unreachable!("every type of TYPES is generated"),
    };
    Case {
        values,
        valid,
        stored,
        numbers,
        strings,
    }
}

/// The statistics of § 5 from the generated numbers: an exact `i128` sum, rounded once
/// by Rust's correctly rounded conversions (`as f64`, and `parse` for a scale).
fn oracle(ty: ColumnType, case: &Case) -> Option<ColumnStats> {
    if !ty.is_numeric() {
        return None;
    }
    let mut nan_count = 0;
    let (mut pos, mut neg) = (false, false);
    let mut exact = 0i128;
    let mut ordered: Vec<(f64, i128, &Vec<u8>)> = Vec::new();
    let picked: Vec<_> = (0..case.valid.len()).filter(|&i| case.valid[i]).collect();
    for &i in &picked {
        match case.numbers[i] {
            Number::Float(x, _) if x.is_nan() => nan_count += 1,
            Number::Float(x, units) => {
                pos |= x == f64::INFINITY;
                neg |= x == f64::NEG_INFINITY;
                exact += units;
                ordered.push((x, 0, &case.stored[i]));
            }
            Number::Int(x) => {
                exact += x;
                ordered.push((0.0, x, &case.stored[i]));
            }
            Number::None => unreachable!("numeric types have numbers"),
        }
    }
    let scale = match ty {
        ColumnType::Decimal128 { scale, .. } | ColumnType::Decimal256 { scale, .. } => scale,
        _ => 0,
    };
    #[allow(clippy::cast_precision_loss)]
    let sum = match (pos, neg) {
        (true, true) => f64::NAN,
        (true, false) => f64::INFINITY,
        (false, true) => f64::NEG_INFINITY,
        _ if matches!(ty, ColumnType::Float32 | ColumnType::Float64) => exact as f64 * pow2(-20),
        _ => format!("{exact}e{}", -i32::from(scale))
            .parse::<f64>()
            .unwrap(),
    };
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let pad = |s: &Vec<u8>| {
        let mut b = [0u8; 32];
        b[..s.len()].copy_from_slice(s);
        b
    };
    let count = picked.len() as u64;
    Some(ColumnStats {
        count,
        null_count: case.valid.len() as u64 - count,
        nan_count,
        sum: if sum == 0.0 { 0.0 } else { sum },
        range: ordered
            .first()
            .zip(ordered.last())
            .map(|(lo, hi)| (pad(lo.2), pad(hi.2))),
    })
}

/// Writes a generated column in batches of `batch_rows` (each given as two slices when
/// `split`) and reads it back.
fn run_case(
    ty: ColumnType,
    rows: usize,
    batch_rows: u64,
    nullable: bool,
    seed: u64,
    split: bool,
) -> (Case, Read) {
    let case = generate(ty, rows, seed, nullable);
    let strings: Vec<&str> = case.strings.iter().map(String::as_str).collect();
    let (offsets, bytes) = Dictionary::parts(&strings);
    let spec = ColumnSpec {
        nullable,
        batch_rows,
        dictionary: matches!(ty, ColumnType::Dictionary(_)).then_some(Dictionary {
            offsets: &offsets,
            bytes: &bytes,
        }),
        ..ColumnSpec::new("c", ty, rows as u64)
    };
    // One bitmap of the whole column; each batch's validity is cut from it at bit
    // offsets, in two pieces when `split`.
    let mut bitmap = vec![0u8; rows.div_ceil(8)];
    for (i, &v) in case.valid.iter().enumerate() {
        bitmap[i / 8] |= u8::from(v) << (i % 8);
    }
    let piece = |r: std::ops::Range<usize>| Bits {
        bytes: &bitmap,
        offset: r.start as u64,
        len: r.len() as u64,
    };
    let r = usize::try_from(batch_rows).unwrap();
    let batches: Vec<(Vec<Values<'_>>, Vec<Bits<'_>>)> = (0..rows.div_ceil(r))
        .map(|b| {
            let range = b * r..rows.min(b * r + r);
            let mid = range.start + range.len() / 2;
            let parts = if split {
                vec![range.start..mid, mid..range.end]
            } else {
                vec![range]
            };
            let values = parts.iter().map(|p| case.values.rows(p.clone())).collect();
            let bits = if nullable {
                parts.into_iter().map(piece).collect()
            } else {
                Vec::new()
            };
            (values, bits)
        })
        .collect();
    let read = round_trip(&spec, &batches);
    (case, read)
}

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 512,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn every_type_round_trips(
        t in 0..TYPES.len(),
        rows in 0usize..300,
        r in 0u32..7,
        nullable: bool,
        split: bool,
        seed: u64,
    ) {
        let ty = TYPES[t];
        let (case, read) = run_case(ty, rows, 1 << r, nullable, seed, split);
        let want: Vec<Option<Vec<u8>>> = case
            .valid
            .iter()
            .zip(&case.stored)
            .map(|(&v, s)| v.then(|| s.clone()))
            .collect();
        prop_assert_eq!(&read.rows, &want);
        prop_assert_eq!(read.layout.rows(), rows as u64);
        prop_assert_eq!(read.layout.batches().len(), rows.div_ceil(1 << r));
        for b in read.layout.batches() {
            prop_assert_eq!(b.values.start % 4096, 0);
            prop_assert_eq!(b.validity.start % 4096, 0);
            prop_assert_eq!(b.validity.is_empty(), b.null_count == 0);
        }
        prop_assert_eq!(read.stats.as_ref().map(key), oracle(ty, &case).as_ref().map(key));
        if let Some(d) = read.layout.dictionary() {
            let column = Column::open(&read.layout, Mem::new(read.column.clone())).unwrap();
            let mut buf = Vec::new();
            for (code, s) in (0..d.values).zip(&case.strings) {
                prop_assert_eq!(column.dictionary_entry(code, &mut buf).unwrap(), s.as_str());
            }
            prop_assert!(column.dictionary_entry(d.values, &mut buf).is_err());
        }
    }

    /// Every byte of a column section outside its buffers is checked: changing one
    /// makes the reader refuse the section; changing a byte of a buffer does not.
    #[test]
    fn every_framing_byte_is_checked(
        t in 0..TYPES.len(),
        rows in 1usize..40,
        r in 0u32..4,
        nullable: bool,
        seed: u64,
        at: prop::sample::Index,
        flip in 1u8..=255,
    ) {
        let (_, read) = run_case(TYPES[t], rows, 1 << r, nullable, seed, false);
        let mut column = read.column.clone();
        let i = at.index(column.len());
        column[i] ^= flip;
        let i = i as u64;
        let layout = &read.layout;
        let in_buffer = layout
            .batches()
            .iter()
            .flat_map(|b| [&b.validity, &b.values])
            .chain(layout.dictionary().iter().flat_map(|d| [&d.offsets, &d.data]))
            .any(|range| range.contains(&i));
        prop_assert_eq!(Column::open(layout, Mem::new(column)).is_ok(), in_buffer);
    }

    /// A damaged layout is refused by its parser or by the column check, or reads the
    /// same rows: the batch row count of a column of one batch is not in its Arrow file,
    /// and any power of two from its rows up describes the same bytes.
    #[test]
    fn a_damaged_layout_is_refused(
        t in 0..TYPES.len(),
        rows in 1usize..40,
        r in 0u32..4,
        seed: u64,
        at: prop::sample::Index,
        flip in 1u8..=255,
    ) {
        let (_, read) = run_case(TYPES[t], rows, 1 << r, true, seed, false);
        let mut bytes = read.layout_bytes.clone();
        let i = at.index(bytes.len());
        bytes[i] ^= flip;
        let refused = match ColumnLayout::parse(&bytes) {
            Err(_) => true,
            Ok(layout) => {
                let opened = Column::open(&layout, Mem::new(read.column.clone())).is_ok();
                let same = layout.batches().len() == 1
                    && layout.batches() == read.layout.batches()
                    && stored_rows(&layout, &read.column) == read.rows;
                !opened || same
            }
        };
        prop_assert!(refused);
    }

    /// The float sum, over the whole exponent range (subnormals, values near
    /// `f64::MAX` that overflow, exact cancellations), is the exact sum rounded once:
    /// the one an independent fixed-point reference computes, in any order.
    #[test]
    fn float_sums_are_exact(seed: u64, n in 0usize..300) {
        let mut rng = SplitMix64::new(seed);
        let mut xs: Vec<f64> = Vec::new();
        while xs.len() < n {
            let r = rng.next_u64();
            let x = match r % 8 {
                0 => f64::from_bits(r >> 12),                       // subnormal or tiny
                1 => f64::MAX * f64::from_bits(0x3FF0_0000_0000_0000 | (r >> 12)) / 2.0, // huge
                2 => xs.last().map_or(1.0, |y| -y),                   // cancels the last
                _ => f64::from_bits(r),
            };
            if x.is_finite() {
                xs.push(x);
            }
        }
        let want = reference::sum(&xs);
        prop_assert_eq!(float_sum(&xs).to_bits(), want.to_bits());
        xs.reverse();
        prop_assert_eq!(float_sum(&xs).to_bits(), want.to_bits());
    }

    /// Integers and decimals: the sum is Rust's correctly rounded parse of the exact
    /// `i128` sum with the scale as exponent.
    #[test]
    fn decimal_sums_round_once(
        xs in prop::collection::vec(any::<i64>(), 0..50),
        scale in -20i8..=40,
    ) {
        let values: Vec<i128> = xs.iter().map(|&x| i128::from(x) * 1_000_003).collect();
        let total: i128 = values.iter().sum();
        let want: f64 = format!("{total}e{}", -i32::from(scale)).parse().unwrap();
        prop_assert_eq!(decimal_sum(&values, scale).to_bits(), (want + 0.0).to_bits());
    }
}

/// The statistics sum of a `float64` column of `xs`.
fn float_sum(xs: &[f64]) -> f64 {
    let mut w = Writer::create(Mem::new(Vec::new())).unwrap();
    let spec = ColumnSpec::new("s", ColumnType::Float64, xs.len() as u64);
    write_column(&mut w, "t", &spec, xs, None)
        .unwrap()
        .unwrap()
        .sum
}

/// The statistics sum of a `decimal128(38, scale)` column of `xs`.
fn decimal_sum(xs: &[i128], scale: i8) -> f64 {
    let mut w = Writer::create(Mem::new(Vec::new())).unwrap();
    let ty = ColumnType::Decimal128 {
        precision: 38,
        scale,
    };
    let spec = ColumnSpec::new("s", ty, xs.len() as u64);
    write_column(&mut w, "t", &spec, xs, None)
        .unwrap()
        .unwrap()
        .sum
}

/// An exact sum of finite `f64` by another method than the writer's: each value as an
/// integer multiple of 2^−1074 in a `Vec` of decimal digits of base 10^18, positive and
/// negative parts apart; the difference is printed in decimal and parsed by Rust's
/// correctly rounded `str::parse`, with the exponent −1074 applied as a power of 2
/// through an exact decimal expansion of 2^−1074.
mod reference {
    const BASE: u128 = 1_000_000_000_000_000_000;

    /// `a += b` for numbers of base-10^18 digits, least significant first.
    fn add(a: &mut Vec<u128>, b: &[u128]) {
        let mut carry = 0;
        for i in 0..a.len().max(b.len()) {
            if i == a.len() {
                a.push(0);
            }
            let x = a[i] + b.get(i).copied().unwrap_or(0) + carry;
            a[i] = x % BASE;
            carry = x / BASE;
        }
        if carry > 0 {
            a.push(carry);
        }
    }

    fn mul(a: &mut Vec<u128>, k: u128) {
        let mut carry = 0;
        for d in a.iter_mut() {
            let x = *d * k + carry;
            *d = x % BASE;
            carry = x / BASE;
        }
        while carry > 0 {
            a.push(carry % BASE);
            carry /= BASE;
        }
    }

    fn cmp(a: &[u128], b: &[u128]) -> std::cmp::Ordering {
        let trim = |x: &[u128]| x.len() - x.iter().rev().take_while(|&&d| d == 0).count();
        let (la, lb) = (trim(a), trim(b));
        la.cmp(&lb)
            .then_with(|| a[..la].iter().rev().cmp(b[..lb].iter().rev()))
    }

    /// `a − b` for `a ≥ b`.
    fn sub(a: &[u128], b: &[u128]) -> Vec<u128> {
        let mut out = Vec::with_capacity(a.len());
        let mut borrow = 0;
        for (i, &x) in a.iter().enumerate() {
            let y = b.get(i).copied().unwrap_or(0) + borrow;
            if x >= y {
                out.push(x - y);
                borrow = 0;
            } else {
                out.push(x + BASE - y);
                borrow = 1;
            }
        }
        out
    }

    fn decimal(a: &[u128]) -> String {
        let mut digits = a.iter().rev().skip_while(|&&d| d == 0);
        let Some(first) = digits.next() else {
            return "0".into();
        };
        let mut s = first.to_string();
        for d in digits {
            s.push_str(&format!("{d:018}"));
        }
        s
    }

    /// The exact sum of `xs`, all finite, rounded once to the nearest `f64`; +0.0 for
    /// zero, ±∞ past `f64::MAX` after rounding.
    pub(super) fn sum(xs: &[f64]) -> f64 {
        let (mut pos, mut neg) = (vec![0u128], vec![0u128]);
        for &x in xs {
            let bits = x.to_bits();
            let (exp, frac) = ((bits >> 52) & 0x7FF, bits & ((1 << 52) - 1));
            let (m, shift) = if exp == 0 {
                (frac, 0)
            } else {
                (frac | 1 << 52, exp - 1)
            };
            let mut v = vec![u128::from(m)];
            for _ in 0..shift {
                mul(&mut v, 2);
            }
            add(
                if x.is_sign_negative() {
                    &mut neg
                } else {
                    &mut pos
                },
                &v,
            );
        }
        let (negative, m) = match cmp(&pos, &neg) {
            std::cmp::Ordering::Equal => return 0.0,
            std::cmp::Ordering::Greater => (false, sub(&pos, &neg)),
            std::cmp::Ordering::Less => (true, sub(&neg, &pos)),
        };
        // m × 2^−1074 = m × 5^1074 × 10^−1074, an exact decimal.
        let mut d = m;
        for _ in 0..1074 {
            mul(&mut d, 5);
        }
        let x: f64 = format!("{}e-1074", decimal(&d)).parse().unwrap();
        if negative { -x } else { x + 0.0 }
    }
}

// ---------------------------------------------------------------------------------------
// The exact sum on the cases that defeat a running `f64` sum.

#[test]
fn hard_float_sums() {
    let exact = float_sum;
    let tiny = f64::from_bits(1);
    let ulp = f64::EPSILON;
    assert_eq!(exact(&[1e308, 1e308, -1e308]), 1e308);
    assert_eq!(exact(&[1e300, 1.0, -1e300]), 1.0);
    assert_eq!(exact(&[tiny, tiny, tiny]).to_bits(), 3);
    // Ties go to even; anything past the tie goes up.
    assert_eq!(exact(&[1.0, ulp / 2.0]), 1.0);
    assert_eq!(exact(&[1.0, ulp / 2.0, tiny]), 1.0 + ulp);
    assert_eq!(exact(&[1.0 + ulp, ulp / 2.0]), 1.0 + 2.0 * ulp);
    // Overflow, also by rounding up at the tie just past f64::MAX.
    assert_eq!(exact(&[f64::MAX, f64::MAX]), f64::INFINITY);
    assert_eq!(exact(&[-f64::MAX, -f64::MAX]), f64::NEG_INFINITY);
    assert_eq!(exact(&[f64::MAX, pow2(970)]), f64::INFINITY);
    assert_eq!(exact(&[f64::MAX, pow2(969)]), f64::MAX);
    // Subnormal results, and the carry into the smallest normal.
    assert_eq!(exact(&[f64::MIN_POSITIVE, -tiny]).to_bits(), (1 << 52) - 1);
    let half = f64::MIN_POSITIVE / 2.0;
    assert_eq!(exact(&[half, half]), f64::MIN_POSITIVE);
    // Zero is +0.0, never −0.0; infinities.
    assert_eq!(exact(&[-0.0]).to_bits(), 0);
    assert_eq!(exact(&[-tiny, tiny]).to_bits(), 0);
    assert_eq!(exact(&[]).to_bits(), 0);
    assert_eq!(exact(&[f64::INFINITY, 1.0]), f64::INFINITY);
    assert!(exact(&[f64::INFINITY, f64::NEG_INFINITY]).is_nan());
    // Ten times 0.1 is exactly 1.000000000000000055511151231257827…, which rounds to 1.
    assert_eq!(exact(&[0.1; 10]), 1.0);
    // The reference agrees on all of these.
    for xs in [
        &[1e308, 1e308, -1e308][..],
        &[1.0, ulp / 2.0, tiny],
        &[f64::MIN_POSITIVE, -tiny],
        &[0.1; 10],
    ] {
        assert_eq!(reference::sum(xs).to_bits(), exact(xs).to_bits());
    }
}

#[test]
fn decimal_sums() {
    assert_eq!(decimal_sum(&[1, 2], 1), 0.3);
    assert_eq!(decimal_sum(&[1, 2], -2), 300.0);
    // ±(10^76 − 1), the extremes of decimal256(76, s), at scale 18.
    let extreme = |negative: bool| {
        let (hi, lo) = (
            0x161b_cca7_1199_15b5_0764_b4ab_e865_2979_i128,
            0x7775_a5f1_7195_0fff_ffff_ffff_ffff_ffff_u128,
        );
        let (hi, lo) = if negative { (!hi, !lo + 1) } else { (hi, lo) };
        [lo.to_le_bytes(), hi.to_le_bytes()].concat()
    };
    let sum = |parts: &[Vec<u8>]| {
        let mut w = Writer::create(Mem::new(Vec::new())).unwrap();
        let ty = ColumnType::Decimal256 {
            precision: 76,
            scale: 18,
        };
        let bytes = parts.concat();
        let spec = ColumnSpec::new("s", ty, parts.len() as u64);
        write_column(&mut w, "t", &spec, Values::Bytes(&bytes), None)
            .unwrap()
            .unwrap()
            .sum
    };
    assert_eq!(sum(&[extreme(false)]), 1e58);
    assert_eq!(sum(&[extreme(false), extreme(true), extreme(true)]), -1e58);
}

// ---------------------------------------------------------------------------------------
// What the writer refuses; types and names.

fn input<T: std::fmt::Debug>(r: Result<T, ColumnError>) -> bool {
    matches!(r, Err(ColumnError::Input(_)))
}

#[test]
fn the_writer_refuses_what_breaks_the_spec() {
    let mut w = Writer::create(Mem::new(Vec::new())).unwrap();
    let spec = ColumnSpec {
        batch_rows: 2,
        ..ColumnSpec::new("a", ColumnType::Int32, 3)
    };
    let mut c = ColumnWriter::begin(&mut w, "nodes", &spec, None).unwrap();
    assert!(input(c.write_batch(&[Values::I64(&[1, 2])], &[])));
    assert!(input(c.write_batch(&[Values::I32(&[1])], &[])));
    let bits = Bits {
        bytes: &[3],
        offset: 0,
        len: 2,
    };
    assert!(input(c.write_batch(&[Values::I32(&[1, 2])], &[bits])));
    c.write_batch(&[Values::I32(&[1]), Values::I32(&[2])], &[])
        .unwrap();
    assert!(input(c.finish()));

    let base = ColumnSpec::new("b", ColumnType::Int32, 3);
    let begin = |spec: ColumnSpec<'_>, w: &mut Writer<Mem>| {
        ColumnWriter::begin(w, "nodes", &spec, None).map(|_| ())
    };
    assert!(input(begin(
        ColumnSpec {
            batch_rows: 3,
            ..base
        },
        &mut w
    )));
    assert!(input(begin(
        ColumnSpec {
            batch_rows: 1 << 31,
            ..base
        },
        &mut w
    )));
    assert!(input(begin(ColumnSpec { name: "", ..base }, &mut w)));
    let decimal = ColumnType::Decimal128 {
        precision: 39,
        scale: 0,
    };
    assert!(input(begin(
        ColumnSpec {
            ty: decimal,
            ..base
        },
        &mut w
    )));
    let codes = ColumnType::Dictionary(CodeWidth::U8);
    assert!(input(begin(ColumnSpec { ty: codes, ..base }, &mut w)));
    let (offsets, bytes) = Dictionary::parts(&vec!["x"; 257]);
    let many = Dictionary {
        offsets: &offsets,
        bytes: &bytes,
    };
    assert!(input(begin(
        ColumnSpec {
            ty: codes,
            dictionary: Some(many),
            ..base
        },
        &mut w
    )));
    assert!(input(begin(
        ColumnSpec {
            dictionary: Some(many),
            ..base
        },
        &mut w
    )));
    // One row per batch over 2^40 rows: a layout past 16 MiB.
    assert!(matches!(
        begin(
            ColumnSpec {
                rows: 1 << 40,
                batch_rows: 1,
                ..base
            },
            &mut w
        ),
        Err(ColumnError::TooLarge(_))
    ));

    let (offsets, bytes) = Dictionary::parts(&["a", "b"]);
    let ab = Dictionary {
        offsets: &offsets,
        bytes: &bytes,
    };
    let spec = ColumnSpec {
        dictionary: Some(ab),
        ..ColumnSpec::new("k", codes, 2)
    };
    assert!(input(write_column(&mut w, "edges", &spec, &[0u8, 2], None)));
    // Offsets that do not end at the bytes' length, or strings that are not UTF-8.
    for (offsets, bytes) in [
        (&[0u32, 1, 3][..], &b"abcd"[..]),
        (&[0, 1, 2], &[b'a', 0xFF]),
    ] {
        let spec = ColumnSpec {
            dictionary: Some(Dictionary { offsets, bytes }),
            ..ColumnSpec::new("k2", codes, 0)
        };
        assert!(input(write_column(&mut w, "edges", &spec, &[0u8; 0], None)));
    }
    // A scope that holds a '/', or validity past its bytes.
    let spec = ColumnSpec::new("p", ColumnType::Int32, 1);
    assert!(input(write_column(&mut w, "a/arrow", &spec, &[1i32], None)));
    let spec = ColumnSpec {
        nullable: true,
        ..ColumnSpec::new("q", ColumnType::Int32, 1)
    };
    let mut c = ColumnWriter::begin(&mut w, "nodes", &spec, None).unwrap();
    let past = Bits {
        bytes: &[1],
        offset: 8,
        len: 1,
    };
    assert!(input(c.write_batch(&[Values::I32(&[1])], &[past])));
    drop(c);
    let ty = ColumnType::Decimal128 {
        precision: 3,
        scale: 0,
    };
    let spec = ColumnSpec::new("d", ty, 2);
    assert!(input(write_column(
        &mut w,
        "edges",
        &spec,
        &[999i128, -1000],
        None
    )));
    let spec = ColumnSpec::new("e", ty, 2);
    write_column(&mut w, "edges", &spec, &[999i128, -999], None).unwrap();
    assert!(matches!(
        write_column(&mut w, "edges", &spec, &[1i128, 2], None),
        Err(ColumnError::Container(ContainerError::NameTaken(_)))
    ));
}

/// A dictionary layout whose string bytes, added up by the reader, would pass 2^64
/// (review of #213): refused, not a panic or an inverted range.
#[test]
fn a_dictionary_past_two_gib_is_refused() {
    let mut b = b"EGCOLLAY".to_vec();
    b.extend_from_slice(&1u32.to_le_bytes()); // version
    b.extend_from_slice(&0u32.to_le_bytes()); // flags
    for x in [0u64, 1, 0, 4330] {
        b.extend_from_slice(&x.to_le_bytes()); // rows, batch rows, batches, column bytes
    }
    b.extend_from_slice(&[8, 0, 0, 0]); // dictionary, not nullable
    for x in [8u32, 0, 1] {
        b.extend_from_slice(&x.to_le_bytes()); // uint8 codes, parameter 2, name length
    }
    b.extend_from_slice(&1u64.to_le_bytes()); // strings
    b.extend_from_slice(&(u64::MAX - 4099).to_le_bytes()); // string bytes
    b.extend_from_slice(b"x\0\0\0\0\0\0\0");
    b.resize(120, 0);
    assert!(matches!(
        ColumnLayout::parse(&b),
        Err(ColumnError::Invalid(_))
    ));
}

#[test]
fn types_and_names() {
    assert_eq!(ColumnType::edge_id((1 << 32) - 1), ColumnType::UInt32);
    assert_eq!(ColumnType::edge_id(1 << 32), ColumnType::UInt64);
    assert_eq!(ColumnType::Float32.default_batch_rows(), 1 << 26);
    assert_eq!(ColumnType::Bool.default_batch_rows(), 1 << 26);
    let wide = ColumnType::Decimal256 {
        precision: 76,
        scale: 0,
    };
    assert_eq!(wide.default_batch_rows(), 1 << 25);
    assert_eq!(
        ColumnType::FixedSizeBinary(1 << 20).default_batch_rows(),
        1 << 10
    );
    let n = SectionNames::new("adjacency.out.pair", "weight");
    assert_eq!(n.column, "adjacency.out.pair/arrow/weight");
    assert_eq!(n.layout, "adjacency.out.pair/layout/weight");
    assert_eq!(n.stats, "adjacency.out.pair/stats/weight");
}

#[test]
fn clean_columns() {
    let stats = |range: Option<(f64, f64)>, nan_count, sum| {
        let b = |x: f64| {
            let mut out = [0; 32];
            out[..8].copy_from_slice(&x.to_le_bytes());
            out
        };
        ColumnStats {
            count: 3,
            null_count: 0,
            nan_count,
            sum,
            range: range.map(|(a, z)| (b(a), b(z))),
        }
    };
    let f = ColumnType::Float64;
    assert!(stats(Some((0.0, 2.0)), 0, 3.0).is_clean(f));
    assert!(stats(None, 0, 0.0).is_clean(f));
    assert!(!stats(Some((-1.0, 2.0)), 0, 3.0).is_clean(f));
    assert!(!stats(Some((0.0, 2.0)), 1, 3.0).is_clean(f));
    assert!(!stats(Some((0.0, f64::INFINITY)), 0, f64::INFINITY).is_clean(f));
    let mut min = [0u8; 32];
    min[..8].copy_from_slice(&(-5i64).to_le_bytes());
    let int = |sum| ColumnStats {
        count: 1,
        null_count: 0,
        nan_count: 0,
        sum,
        range: Some((min, min)),
    };
    assert!(!int(-5.0).is_clean(ColumnType::Int64));
    assert!(int(5.0).is_clean(ColumnType::UInt64));
}
