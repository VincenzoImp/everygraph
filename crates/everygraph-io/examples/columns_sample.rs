// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The Rust side of `tools/check_columns_python.py`, which reads what it writes with
//! `pyarrow` and Polars, without Everygraph:
//!
//! - `write PATH` writes a sample `.eg` file of property columns of every type, with
//!   nulls, NaNs, −0.0, infinities, extreme integers and several batches, and prints
//!   as JSON what each column holds (the values as given to the writer);
//! - `write PATH --big` writes instead the check of `docs/research/sota-ingest-query.md`
//!   C4: a `float32` column of 2^27 rows (two batches; row `i` holds `i as f32`) and a
//!   `uint8` dictionary column of 2^20 rows (row `i` holds code `i % 3` of
//!   `["a", "bb", "ccc"]`).
//!
//! ```sh
//! cargo run -p everygraph-io --example columns_sample -- write sample.eg
//! ```

#![forbid(unsafe_code)]

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use everygraph_format::columns::{
    Bits, CodeWidth, ColumnSpec, ColumnType, ColumnWriter, Dictionary, TimeUnit, Values,
    write_column,
};
use everygraph_format::container::{Provenance, Writer};
use everygraph_io::{FileSink, persist};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// One column of the sample: written, and described in the JSON.
struct Sample {
    json: String,
}

impl Sample {
    fn column(&mut self, scope: &str, spec: &ColumnSpec<'_>, values: &str) {
        let sep = if self.json.is_empty() { "" } else { ",\n" };
        let _ = write!(
            self.json,
            "{sep}{{\"scope\": \"{scope}\", \"name\": \"{}\", \"type\": \"{:?}\", \"nullable\": {}, \"batch_rows\": {}, \"rows\": {}, \"values\": [{values}]}}",
            spec.name, spec.ty, spec.nullable, spec.batch_rows, spec.rows,
        );
    }
}

/// Writes a nullable column batch by batch; `None` rows are null.
fn nullable<'a, T: Copy + Default + 'a>(
    w: &mut Writer<FileSink>,
    scope: &str,
    spec: &ColumnSpec<'_>,
    rows: &'a [Option<T>],
    values: impl Fn(&[T]) -> Values<'_>,
) -> Result<()> {
    let mut column = ColumnWriter::begin(w, scope, spec, Some("provenance/ingest"))?;
    for batch in rows.chunks(usize::try_from(spec.batch_rows)?) {
        let plain: Vec<T> = batch.iter().map(|x| x.unwrap_or_default()).collect();
        let mut bits = vec![0u8; batch.len().div_ceil(8)];
        for (i, x) in batch.iter().enumerate() {
            bits[i / 8] |= u8::from(x.is_some()) << (i % 8);
        }
        let validity = Bits {
            bytes: &bits,
            offset: 0,
            len: batch.len() as u64,
        };
        column.write_batch(&[values(&plain)], &[validity])?;
    }
    column.finish()?;
    Ok(())
}

fn list<T>(rows: &[Option<T>], show: impl Fn(&T) -> String) -> String {
    rows.iter()
        .map(|x| x.as_ref().map_or("null".to_owned(), &show))
        .collect::<Vec<_>>()
        .join(", ")
}

fn small(w: &mut Writer<FileSink>, s: &mut Sample) -> Result<()> {
    let ingest = Some("provenance/ingest");
    let bits = |x: &f64| format!("\"{:#x}\"", x.to_bits());

    let weight = [
        Some(0.5),
        Some(-0.0),
        Some(f64::from_bits(0x7FF0_0000_0000_0001)),
        Some(1e308),
        Some(1e308),
        Some(-1e308),
        None,
        Some(5e-324),
        Some(3.25),
        None,
        Some(0.1),
    ];
    let spec = ColumnSpec {
        nullable: true,
        batch_rows: 4,
        ..ColumnSpec::new("weight", ColumnType::Float64, weight.len() as u64)
    };
    nullable(w, "adjacency.out.pair", &spec, &weight, |v| Values::F64(v))?;
    s.column("adjacency.out.pair", &spec, &list(&weight, bits));

    let time: Vec<Option<i64>> = (0..20)
        .map(|i: i64| (i % 7 != 3).then_some(1_700_000_000_000 + i * 86_400_000 - (i % 5) * 1000))
        .collect();
    let spec = ColumnSpec {
        nullable: true,
        batch_rows: 8,
        ..ColumnSpec::new("time", ColumnType::Timestamp(TimeUnit::Millisecond), 20)
    };
    nullable(w, "adjacency.out.event", &spec, &time, |v| Values::I64(v))?;
    s.column("adjacency.out.event", &spec, &list(&time, i64::to_string));

    let ids: Vec<u32> = (0..20).rev().collect();
    let spec = ColumnSpec {
        batch_rows: 16,
        ..ColumnSpec::new("edge_id", ColumnType::edge_id(20), 20)
    };
    write_column(w, "adjacency.out.event", &spec, &ids[..], ingest)?;
    let shown: Vec<_> = ids.iter().map(|x| Some(*x)).collect();
    s.column("adjacency.out.event", &spec, &list(&shown, u32::to_string));

    let strings = ["swap", "transfer", "mint", ""];

    let (offsets, bytes) = Dictionary::parts(&strings);
    let kind = [
        Some(0u8),
        Some(1),
        None,
        Some(3),
        Some(2),
        Some(0),
        Some(0),
        None,
        Some(1),
        Some(3),
    ];
    let spec = ColumnSpec {
        nullable: true,
        batch_rows: 4,
        dictionary: Some(Dictionary {
            offsets: &offsets,
            bytes: &bytes,
        }),
        ..ColumnSpec::new(
            "kind",
            ColumnType::Dictionary(CodeWidth::U8),
            kind.len() as u64,
        )
    };
    nullable(w, "edges", &spec, &kind, |v| Values::U8(v))?;
    s.column(
        "edges",
        &spec,
        &list(&kind, |c| format!("\"{}\"", strings[usize::from(*c)])),
    );

    // decimal256(76, 18): the stored integers as (high 128 bits, low 128 bits), among
    // them ±(10^76 − 1), the largest a precision of 76 allows.
    let (hi76, lo76) = (
        0x161b_cca7_1199_15b5_0764_b4ab_e865_2979_i128,
        0x7775_a5f1_7195_0fff_ffff_ffff_ffff_ffff_u128,
    );
    let amount: Vec<Option<(i128, u128)>> = vec![
        Some((0, 1_000_000_000_000_000_000)),
        Some((-1, u128::MAX - 999)),
        None,
        Some((hi76, lo76)),
        Some((!hi76, !lo76 + 1)),
        Some((0, 42)),
        Some((12_345, 678)),
    ];
    let spec = ColumnSpec {
        nullable: true,
        batch_rows: 2,
        ..ColumnSpec::new(
            "amount",
            ColumnType::Decimal256 {
                precision: 76,
                scale: 18,
            },
            amount.len() as u64,
        )
    };
    let le = |&(hi, lo): &(i128, u128)| {
        let mut b = [0u8; 32];
        b[..16].copy_from_slice(&lo.to_le_bytes());
        b[16..].copy_from_slice(&hi.to_le_bytes());
        b
    };
    let rows: Vec<Option<[u8; 32]>> = amount.iter().map(|x| x.as_ref().map(le)).collect();
    nullable(w, "edges", &spec, &rows, |v: &[[u8; 32]]| {
        Values::Bytes(v.as_flattened())
    })?;
    s.column(
        "edges",
        &spec,
        &list(&amount, |(hi, lo)| format!("[{hi}, {lo}]")),
    );

    let fee = [
        12_345i128,
        -99_999_999_999_999_999_999 / 10_000_000,
        0,
        1,
        7_000_000,
    ];
    let spec = ColumnSpec::new(
        "fee",
        ColumnType::Decimal128 {
            precision: 20,
            scale: 4,
        },
        5,
    );
    write_column(w, "edges", &spec, &fee[..], ingest)?;
    let shown: Vec<_> = fee.iter().map(|x| Some(*x)).collect();
    s.column("edges", &spec, &list(&shown, i128::to_string));

    let flag: Vec<Option<bool>> = (0..19)
        .map(|i| (i % 4 != 2).then_some(i % 3 == 0))
        .collect();
    let spec = ColumnSpec {
        nullable: true,
        batch_rows: 8,
        ..ColumnSpec::new("flag", ColumnType::Bool, 19)
    };
    nullable(w, "nodes", &spec, &flag, |v| Values::Bool(v))?;
    s.column("nodes", &spec, &list(&flag, bool::to_string));

    let hash: Vec<u8> = (0..60u8).map(|b| b.wrapping_mul(37)).collect();
    let spec = ColumnSpec::new("hash", ColumnType::FixedSizeBinary(20), 3);
    write_column(w, "nodes", &spec, Values::Bytes(&hash), ingest)?;
    let shown: Vec<_> = hash.chunks(20).map(Some).collect();
    s.column(
        "nodes",
        &spec,
        &list(&shown, |h| {
            format!(
                "\"{}\"",
                h.iter().map(|b| format!("{b:02x}")).collect::<String>()
            )
        }),
    );

    let count = [i64::MIN, i64::MAX, 0, -1, 7];
    let spec = ColumnSpec::new("count", ColumnType::Int64, 5);
    write_column(w, "nodes", &spec, &count[..], ingest)?;
    s.column("nodes", &spec, &list(&count.map(Some), i64::to_string));

    let total = [u64::MAX, u64::MAX, 1];
    let spec = ColumnSpec::new("total", ColumnType::UInt64, 3);
    write_column(w, "nodes", &spec, &total[..], ingest)?;
    s.column("nodes", &spec, &list(&total.map(Some), u64::to_string));

    // Finite floats over the whole exponent range, subnormals and values near f64::MAX
    // included, each followed by its opposite one time in four: the sum's exactness
    // across cancellations and intermediate overflow.
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut spread = Vec::new();
    while spread.len() < 400 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let x = f64::from_bits(state ^ (state >> 29));
        if x.is_finite() {
            spread.push(x);
            if state.is_multiple_of(4) {
                spread.push(-x);
            }
        }
    }
    let spec = ColumnSpec::new("spread", ColumnType::Float64, spread.len() as u64);
    write_column(w, "nodes", &spec, &spread[..], ingest)?;
    let shown: Vec<_> = spread.iter().map(|x| Some(*x)).collect();
    s.column("nodes", &spec, &list(&shown, bits));

    let score = [1.5f32, f32::NEG_INFINITY, f32::NAN, 0.1, -0.0];
    let spec = ColumnSpec::new("score", ColumnType::Float32, 5);
    write_column(w, "nodes", &spec, &score[..], ingest)?;
    s.column(
        "nodes",
        &spec,
        &list(&score.map(Some), |x| format!("\"{:#x}\"", x.to_bits())),
    );

    let level = [-128i8, 127, 0, -1];
    let spec = ColumnSpec::new("level", ColumnType::Int8, 4);
    write_column(w, "nodes", &spec, &level[..], ingest)?;
    s.column("nodes", &spec, &list(&level.map(Some), i8::to_string));

    let port = [65_535u16, 0, 443];
    let spec = ColumnSpec::new("port", ColumnType::UInt16, 3);
    write_column(w, "nodes", &spec, &port[..], ingest)?;
    s.column("nodes", &spec, &list(&port.map(Some), u16::to_string));

    let spec = ColumnSpec::new("empty", ColumnType::Int32, 0);
    write_column(w, "nodes", &spec, &[0i32; 0][..], ingest)?;
    s.column("nodes", &spec, "");
    golden(w, s)
}

/// The three columns of the golden test (`crates/everygraph-format/tests/columns.rs`,
/// `golden_bytes`), in scope `golden`: the check script compares their sections' hashes
/// with the test's, so the bytes pinned there are bytes pyarrow has read.
fn golden(w: &mut Writer<FileSink>, s: &mut Sample) -> Result<()> {
    let spec = ColumnSpec::new("x", ColumnType::Int32, 3);
    write_column(w, "golden", &spec, &[1i32, -2, 3], None)?;
    s.column("golden", &spec, "1, -2, 3");

    let spec = ColumnSpec {
        nullable: true,
        batch_rows: 2,
        ..ColumnSpec::new("weight", ColumnType::Float64, 3)
    };
    let mut c = ColumnWriter::begin(w, "golden", &spec, None)?;
    let bits = Bits {
        bytes: &[0b01],
        offset: 0,
        len: 2,
    };
    c.write_batch(&[Values::F64(&[0.5, -0.0])], &[bits])?;
    c.write_batch(&[Values::F64(&[f64::NAN])], &[])?;
    c.finish()?;
    let values = format!(
        "\"{:#x}\", null, \"{:#x}\"",
        0.5f64.to_bits(),
        f64::NAN.to_bits()
    );
    s.column("golden", &spec, &values);

    let strings = ["swap", "mint"];

    let (offsets, bytes) = Dictionary::parts(&strings);
    let spec = ColumnSpec {
        dictionary: Some(Dictionary {
            offsets: &offsets,
            bytes: &bytes,
        }),
        ..ColumnSpec::new("kind", ColumnType::Dictionary(CodeWidth::U16), 4)
    };
    write_column(w, "golden", &spec, &[1u16, 0, 0, 1], None)?;
    s.column("golden", &spec, "\"mint\", \"swap\", \"swap\", \"mint\"");
    Ok(())
}

fn big(w: &mut Writer<FileSink>, s: &mut Sample) -> Result<()> {
    let rows = 1u64 << 27;
    let spec = ColumnSpec::new("f32", ColumnType::Float32, rows);
    let mut column = ColumnWriter::begin(w, "big", &spec, Some("provenance/ingest"))?;
    for batch in 0..rows / spec.batch_rows {
        let first = batch * spec.batch_rows;
        #[allow(clippy::cast_precision_loss)]
        let values: Vec<f32> = (first..first + spec.batch_rows).map(|i| i as f32).collect();
        column.write_batch(&[Values::F32(&values)], &[])?;
    }
    column.finish()?;
    s.column("big", &spec, "");

    let strings = ["a", "bb", "ccc"];

    let (offsets, bytes) = Dictionary::parts(&strings);
    let codes: Vec<u8> = (0..1u32 << 20)
        .map(|i| u8::try_from(i % 3).unwrap_or(0))
        .collect();
    let spec = ColumnSpec {
        dictionary: Some(Dictionary {
            offsets: &offsets,
            bytes: &bytes,
        }),
        ..ColumnSpec::new(
            "kind",
            ColumnType::Dictionary(CodeWidth::U8),
            codes.len() as u64,
        )
    };
    write_column(w, "big", &spec, &codes[..], Some("provenance/ingest"))?;
    s.column("big", &spec, "");
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: columns_sample write PATH [--big]";
    let (Some("write"), Some(path)) = (args.first().map(String::as_str), args.get(1)) else {
        return Err(usage.into());
    };
    let path = Path::new(path);
    let partial = path.with_extension("eg.partial");
    let mut w = Writer::create(FileSink::create(&partial)?)?;
    let record = Provenance {
        engine: format!("everygraph {}", env!("CARGO_PKG_VERSION")),
        operation: "columns_sample".into(),
        ..Provenance::default()
    };
    w.add_provenance("provenance/ingest", &record)?;
    let mut sample = Sample {
        json: String::new(),
    };
    if args.get(2).map(String::as_str) == Some("--big") {
        big(&mut w, &mut sample)?;
    } else {
        small(&mut w, &mut sample)?;
    }
    w.commit()?;
    drop(w);
    persist(&partial, path)?;
    let out = format!("{{\"columns\": [\n{}\n]}}\n", sample.json);
    std::io::stdout().lock().write_all(out.as_bytes())?;
    Ok(())
}
