// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tables written to files, in the format their extension names (`--out`,
//! `docs/api-sketch.md` § 5.2): Parquet and Arrow IPC through `arrow-rs`, CSV and
//! JSON by hand. A table is written in batches of rows, so that its size in memory is
//! one batch, whatever the number of rows.
//!
//! The values of each format: integers and floating-point values as they are; a null
//! is Arrow's and Parquet's null, an empty CSV field, a JSON `null`; a floating-point
//! value that is not finite is written as Rust prints it in CSV (`NaN`, `inf`) and as
//! `null` in JSON, which has no such numbers.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;

use arrow_array::{
    ArrayRef, Float32Array, Float64Array, Int64Array, RecordBatch, StringArray, UInt32Array,
    UInt64Array,
};
use arrow_schema::{ArrowError, DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::errors::ParquetError;

/// A file format of tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Apache Parquet: `.parquet`.
    Parquet,
    /// The Arrow IPC file format: `.arrow`, `.feather`, `.ipc`.
    Arrow,
    /// Comma-separated values with a header row (RFC 4180): `.csv`.
    Csv,
    /// One JSON array of objects, one per row: `.json`.
    Json,
}

impl Format {
    /// The extensions, as a message lists them.
    pub const EXTENSIONS: &'static str = ".parquet, .arrow, .csv or .json";

    /// The format that the extension of `path` names, in any case.
    #[must_use]
    pub fn of(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "parquet" | "pq" => Some(Self::Parquet),
            "arrow" | "feather" | "ipc" => Some(Self::Arrow),
            "csv" => Some(Self::Csv),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// The type of a column, and whether it may hold nulls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    /// UTF-8 text.
    Text(bool),
    /// Signed 64-bit integers.
    I64(bool),
    /// Unsigned 64-bit integers.
    U64(bool),
    /// Unsigned 32-bit integers.
    U32(bool),
    /// IEEE 754 binary64.
    F64,
    /// IEEE 754 binary32.
    F32,
}

/// The values of one column in one batch; `None` is a null.
#[derive(Clone, Copy, Debug)]
pub enum Values<'a> {
    /// Text.
    Text(&'a [Option<String>]),
    /// Signed integers.
    I64(&'a [Option<i64>]),
    /// Unsigned integers.
    U64(&'a [Option<u64>]),
    /// Unsigned 32-bit integers; `null` is the value written as null.
    U32(&'a [u32], Option<u32>),
    /// binary64.
    F64(&'a [f64]),
    /// binary32.
    F32(&'a [f32]),
}

impl Values<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Text(v) => v.len(),
            Self::I64(v) => v.len(),
            Self::U64(v) => v.len(),
            Self::U32(v, _) => v.len(),
            Self::F64(v) => v.len(),
            Self::F32(v) => v.len(),
        }
    }

    fn array(&self) -> ArrayRef {
        match *self {
            Self::Text(v) => Arc::new(v.iter().map(Option::as_deref).collect::<StringArray>()),
            Self::I64(v) => Arc::new(Int64Array::from(v.to_vec())),
            Self::U64(v) => Arc::new(UInt64Array::from(v.to_vec())),
            Self::U32(v, None) => Arc::new(UInt32Array::from(v.to_vec())),
            Self::U32(v, Some(null)) => Arc::new(
                v.iter()
                    .map(|&x| (x != null).then_some(x))
                    .collect::<UInt32Array>(),
            ),
            Self::F64(v) => Arc::new(Float64Array::from(v.to_vec())),
            Self::F32(v) => Arc::new(Float32Array::from(v.to_vec())),
        }
    }

    /// Value `i` as CSV (`json` false) or JSON text.
    fn text(&self, i: usize, json: bool, out: &mut String) {
        let null = if json { "null" } else { "" };
        let _ = match *self {
            Self::Text(v) => {
                match &v[i] {
                    None => out.push_str(null),
                    Some(t) if json => json_string(t, out),
                    Some(t) => csv_field(t, out),
                }
                Ok(())
            }
            Self::I64(v) => match v[i] {
                Some(x) => write!(out, "{x}"),
                None => {
                    out.push_str(null);
                    Ok(())
                }
            },
            Self::U64(v) => match v[i] {
                Some(x) => write!(out, "{x}"),
                None => {
                    out.push_str(null);
                    Ok(())
                }
            },
            Self::U32(v, n) if n == Some(v[i]) => {
                out.push_str(null);
                Ok(())
            }
            Self::U32(v, _) => write!(out, "{}", v[i]),
            Self::F64(v) if json && !v[i].is_finite() => {
                out.push_str("null");
                Ok(())
            }
            Self::F64(v) => write!(out, "{}", v[i]),
            Self::F32(v) if json && !v[i].is_finite() => {
                out.push_str("null");
                Ok(())
            }
            Self::F32(v) => write!(out, "{}", v[i]),
        };
    }
}

fn json_string(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn csv_field(text: &str, out: &mut String) {
    if text.contains([',', '"', '\n', '\r']) {
        out.push('"');
        out.push_str(&text.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(text);
    }
}

enum Sink {
    Parquet(ArrowWriter<File>),
    Arrow(arrow_ipc::writer::FileWriter<BufWriter<File>>),
    Text {
        out: BufWriter<File>,
        json: bool,
        rows: u64,
    },
}

/// A table being written to a file.
pub struct TableWriter {
    sink: Sink,
    names: Vec<String>,
    schema: Arc<Schema>,
}

impl std::fmt::Debug for TableWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableWriter")
            .field("names", &self.names)
            .finish_non_exhaustive()
    }
}

fn batch(schema: &Arc<Schema>, columns: &[Values<'_>]) -> io::Result<RecordBatch> {
    let arrays: Vec<ArrayRef> = columns.iter().map(Values::array).collect();
    RecordBatch::try_new(Arc::clone(schema), arrays).map_err(from_arrow)
}

fn other(e: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::other(e)
}

fn from_parquet(e: ParquetError) -> io::Error {
    match e {
        ParquetError::External(e) => match e.downcast::<io::Error>() {
            Ok(e) => *e,
            Err(e) => io::Error::other(e),
        },
        e => other(e),
    }
}

fn from_arrow(e: ArrowError) -> io::Error {
    match e {
        ArrowError::IoError(_, e) => e,
        e => other(e),
    }
}

impl TableWriter {
    /// Creates the table `file` in `format`, with the columns `columns`.
    ///
    /// # Errors
    ///
    /// Writing to the file failed.
    pub fn create(file: File, format: Format, columns: &[(&str, Type)]) -> io::Result<Self> {
        let fields: Vec<Field> = columns
            .iter()
            .map(|&(name, ty)| {
                let (data, nullable) = match ty {
                    Type::Text(nullable) => (DataType::Utf8, nullable),
                    Type::I64(nullable) => (DataType::Int64, nullable),
                    Type::U64(nullable) => (DataType::UInt64, nullable),
                    Type::U32(nullable) => (DataType::UInt32, nullable),
                    Type::F64 => (DataType::Float64, false),
                    Type::F32 => (DataType::Float32, false),
                };
                Field::new(name, data, nullable)
            })
            .collect();
        let schema = Arc::new(Schema::new(fields));
        let names: Vec<String> = columns.iter().map(|(n, _)| (*n).to_owned()).collect();
        let sink = match format {
            Format::Parquet => Sink::Parquet(
                ArrowWriter::try_new(file, Arc::clone(&schema), None).map_err(from_parquet)?,
            ),
            Format::Arrow => Sink::Arrow(
                arrow_ipc::writer::FileWriter::try_new_buffered(file, &schema)
                    .map_err(from_arrow)?,
            ),
            Format::Csv | Format::Json => {
                let json = format == Format::Json;
                let mut out = BufWriter::with_capacity(1 << 20, file);
                if json {
                    out.write_all(b"[")?;
                } else {
                    let mut header = String::new();
                    for (i, name) in names.iter().enumerate() {
                        if i > 0 {
                            header.push(',');
                        }
                        csv_field(name, &mut header);
                    }
                    header.push('\n');
                    out.write_all(header.as_bytes())?;
                }
                Sink::Text { out, json, rows: 0 }
            }
        };
        Ok(Self {
            sink,
            names,
            schema,
        })
    }

    /// Appends one batch: one slice per column, all of the same length, in the order of
    /// [`create`](Self::create).
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidInput`] for columns that do not match the table's;
    /// writing to the file failed.
    pub fn write(&mut self, columns: &[Values<'_>]) -> io::Result<()> {
        let rows = columns.first().map_or(0, Values::len);
        if columns.len() != self.names.len() || columns.iter().any(|c| c.len() != rows) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a batch whose columns are not those of the table",
            ));
        }
        match &mut self.sink {
            Sink::Parquet(w) => w
                .write(&batch(&self.schema, columns)?)
                .map_err(from_parquet),
            Sink::Arrow(w) => w.write(&batch(&self.schema, columns)?).map_err(from_arrow),
            Sink::Text {
                out,
                json,
                rows: written,
            } => {
                let mut line = String::new();
                for i in 0..rows {
                    line.clear();
                    if *json {
                        line.push_str(if *written == 0 { "\n{" } else { ",\n{" });
                        for (k, (name, values)) in self.names.iter().zip(columns).enumerate() {
                            if k > 0 {
                                line.push(',');
                            }
                            json_string(name, &mut line);
                            line.push(':');
                            values.text(i, true, &mut line);
                        }
                        line.push('}');
                    } else {
                        for (k, values) in columns.iter().enumerate() {
                            if k > 0 {
                                line.push(',');
                            }
                            values.text(i, false, &mut line);
                        }
                        line.push('\n');
                    }
                    out.write_all(line.as_bytes())?;
                    *written += 1;
                }
                Ok(())
            }
        }
    }

    /// Writes the end of the table and makes it durable; gives the file back.
    ///
    /// # Errors
    ///
    /// Writing or syncing the file failed.
    pub fn finish(self) -> io::Result<File> {
        let file = match self.sink {
            Sink::Parquet(w) => w.into_inner().map_err(from_parquet)?,
            Sink::Arrow(w) => w
                .into_inner()
                .map_err(from_arrow)?
                .into_inner()
                .map_err(io::IntoInnerError::into_error)?,
            Sink::Text { mut out, json, .. } => {
                if json {
                    out.write_all(b"\n]\n")?;
                }
                out.into_inner().map_err(io::IntoInnerError::into_error)?
            }
        };
        file.sync_all()?;
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_follow_the_extension() {
        assert_eq!(Format::of(Path::new("a/pr.parquet")), Some(Format::Parquet));
        assert_eq!(Format::of(Path::new("pr.Arrow")), Some(Format::Arrow));
        assert_eq!(Format::of(Path::new("pr.csv")), Some(Format::Csv));
        assert_eq!(Format::of(Path::new("pr.json")), Some(Format::Json));
        assert_eq!(Format::of(Path::new("pr.txt")), None);
        assert_eq!(Format::of(Path::new("pr")), None);
    }

    #[test]
    fn text_values_are_escaped() {
        let mut out = String::new();
        csv_field("a,\"b\"", &mut out);
        assert_eq!(out, "\"a,\"\"b\"\"\"");
        out.clear();
        json_string("a\"\n\u{1}", &mut out);
        assert_eq!(out, r#""a\"\n\u0001""#);
        out.clear();
        Values::F64(&[f64::NAN]).text(0, true, &mut out);
        Values::U32(&[7, u32::MAX], Some(u32::MAX)).text(1, true, &mut out);
        Values::U32(&[7, u32::MAX], Some(u32::MAX)).text(0, false, &mut out);
        assert_eq!(out, "nullnull7");
    }
}
