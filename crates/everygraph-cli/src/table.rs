// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A result table — the `node` column, then one value per node — in its three forms
//! (`docs/api-sketch.md` § 5.3): aligned for a terminal, tab-separated when piped, and
//! the rows of the result document; and written whole to a file with `--out`. Columns
//! of nodes (`bfs`'s `parent`) show the nodes' identifiers.

use std::fmt::Write as _;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use everygraph_core::{ByteSource, NodeId, SegVec};
use everygraph_ingest::ids::{ColumnType, Dictionary, DictionaryError, KeyKind};
use everygraph_io::table::{Format, TableWriter, Type, Values};
use everygraph_kernels::run::Column;
use serde_json::Value;

use crate::failure::{self, Failure};
use crate::render::{Style, count};

/// Rows per batch written to a file.
const BATCH: u32 = 1 << 16;
/// Rows shown on a terminal without `--top` (§ 5.3).
pub(crate) const SHOWN: usize = 10;
/// The sign bit, which the dictionary flips in a signed key (`KeyKind::Integers`).
const SIGN: u64 = 1 << 63;

/// How identifiers are written: as the integers they are, or as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Unsigned,
    Signed,
    Text,
}

/// A batch of identifiers; `None` where there is no node.
pub(crate) enum Ids {
    Unsigned(Vec<Option<u64>>),
    Signed(Vec<Option<i64>>),
    Text(Vec<Option<String>>),
}

impl Ids {
    pub(crate) fn values(&self) -> Values<'_> {
        match self {
            Self::Unsigned(v) => Values::U64(v),
            Self::Signed(v) => Values::I64(v),
            Self::Text(v) => Values::Text(v),
        }
    }
}

/// The identifiers of a graph's nodes.
pub(crate) struct Nodes<'a, S> {
    dictionary: Dictionary<'a, S>,
    kind: Kind,
}

impl<'a, S: ByteSource> Nodes<'a, S> {
    pub(crate) fn new(dictionary: Dictionary<'a, S>) -> Self {
        let kind = match (dictionary.kind(), dictionary.column()) {
            (KeyKind::Integers, ColumnType::Int { .. }) => Kind::Signed,
            (KeyKind::Integers, _) => Kind::Unsigned,
            _ => Kind::Text,
        };
        Self { dictionary, kind }
    }

    pub(crate) fn dictionary(&self) -> &Dictionary<'a, S> {
        &self.dictionary
    }

    fn integer(&self, id: u32, buf: &mut Vec<u8>) -> Result<u64, DictionaryError> {
        buf.clear();
        self.dictionary.normalized_key(NodeId::new(id), buf)?;
        let bytes: [u8; 8] = buf
            .as_slice()
            .try_into()
            .map_err(|_| DictionaryError::Corrupt("an integer key that is not 8 bytes"))?;
        Ok(u64::from_be_bytes(bytes))
    }

    /// The identifier of `id`, as text.
    pub(crate) fn text(&self, id: u32) -> Result<String, DictionaryError> {
        Ok(self.dictionary.key(NodeId::new(id))?.to_string())
    }

    /// The identifier of `id`, as JSON: a number for integers.
    pub(crate) fn json(&self, id: u32) -> Result<Value, DictionaryError> {
        let mut buf = Vec::new();
        Ok(match self.kind {
            Kind::Unsigned => self.integer(id, &mut buf)?.into(),
            Kind::Signed => (self.integer(id, &mut buf)? ^ SIGN).cast_signed().into(),
            Kind::Text => self.text(id)?.into(),
        })
    }

    /// The file type of a column of identifiers.
    pub(crate) fn file_type(&self, nullable: bool) -> Type {
        match self.kind {
            Kind::Unsigned => Type::U64(nullable),
            Kind::Signed => Type::I64(nullable),
            Kind::Text => Type::Text(nullable),
        }
    }

    /// The identifiers of `ids`, for a file.
    pub(crate) fn ids(
        &self,
        ids: impl Iterator<Item = Option<u32>>,
    ) -> Result<Ids, DictionaryError> {
        let mut buf = Vec::new();
        Ok(match self.kind {
            Kind::Unsigned => Ids::Unsigned(
                ids.map(|id| id.map(|id| self.integer(id, &mut buf)).transpose())
                    .collect::<Result<_, _>>()?,
            ),
            Kind::Signed => Ids::Signed(
                ids.map(|id| {
                    id.map(|id| Ok((self.integer(id, &mut buf)? ^ SIGN).cast_signed()))
                        .transpose()
                })
                .collect::<Result<_, DictionaryError>>()?,
            ),
            Kind::Text => Ids::Text(
                ids.map(|id| id.map(|id| self.text(id)).transpose())
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
}

/// The values of a column as a slice.
fn slice<T: Copy>(v: &SegVec<T>) -> &[T] {
    v.as_slice().unwrap_or_default()
}

/// Value `i` of a column of integers or nodes, `None` for a null.
fn integer(column: &Column, i: usize) -> Option<u32> {
    match column {
        Column::U32 { values, null } => Some(slice(values)[i]).filter(|v| Some(*v) != *null),
        Column::Nodes { values, null } => Some(slice(values)[i]).filter(|v| v != null),
        Column::F64(_) | Column::F32(_) => None,
    }
}

/// Value `i` of a measure as `f64`, `None` for a null: the order of `--top`.
fn number(column: &Column, i: usize) -> Option<f64> {
    match column {
        Column::U32 { .. } => integer(column, i).map(f64::from),
        Column::F64(v) => Some(slice(v)[i]),
        Column::F32(v) => Some(f64::from(slice(v)[i])),
        Column::Nodes { .. } => None,
    }
}

/// The nodes of the largest `n` values of `column`, largest first, ties in node order,
/// nulls left out (`docs/api-sketch.md` § 2.12).
pub(crate) fn top(column: &Column, rows: usize, n: usize) -> Vec<u32> {
    // Ordered best first: a larger value, or the same value at an earlier node.
    let before = |a: &(f64, u32), b: &(f64, u32)| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1));
    let mut best: Vec<(f64, u32)> = Vec::with_capacity(n.min(rows) + 1);
    for (i, id) in (0..rows).zip(0_u32..) {
        let Some(x) = number(column, i) else { continue };
        if best.len() == n
            && best
                .last()
                .is_none_or(|last| before(&(x, id), last).is_ge())
        {
            continue;
        }
        let at = best.partition_point(|b| before(b, &(x, id)).is_lt());
        best.insert(at, (x, id));
        best.truncate(n);
    }
    best.into_iter().map(|(_, id)| id).collect()
}

/// The rows of one result to show, and whether they are ranked.
pub(crate) struct Shown<'r> {
    pub(crate) columns: &'r [(&'static str, Column)],
    pub(crate) rows: Vec<u32>,
    pub(crate) ranked: bool,
}

impl Shown<'_> {
    /// The column names, as the result document lists them.
    pub(crate) fn names(&self) -> Vec<&'static str> {
        let mut names = vec!["node"];
        names.extend(self.columns.iter().map(|(n, _)| *n));
        if self.ranked {
            names.push("rank");
        }
        names
    }

    /// Each row as `node`, its values and its rank, through `cell`.
    fn each<T>(
        &self,
        node: impl Fn(u32) -> Result<T, DictionaryError>,
        cell: impl Fn(usize, &Column, usize) -> Result<T, DictionaryError>,
        rank: impl Fn(u64) -> T,
    ) -> Result<Vec<Vec<T>>, DictionaryError> {
        self.rows
            .iter()
            .zip(1_u64..)
            .map(|(&id, r)| {
                let mut row = vec![node(id)?];
                for (k, (_, c)) in self.columns.iter().enumerate() {
                    row.push(cell(k, c, id as usize)?);
                }
                if self.ranked {
                    row.push(rank(r));
                }
                Ok(row)
            })
            .collect()
    }

    /// The rows, as the result document holds them.
    pub(crate) fn json_rows<S: ByteSource>(
        &self,
        nodes: &Nodes<'_, S>,
    ) -> Result<Vec<Value>, DictionaryError> {
        let cell = |_, c: &Column, i: usize| -> Result<Value, DictionaryError> {
            Ok(match c {
                Column::Nodes { .. } => match integer(c, i) {
                    Some(id) => nodes.json(id)?,
                    None => Value::Null,
                },
                Column::U32 { .. } => integer(c, i).map_or(Value::Null, Value::from),
                Column::F64(_) | Column::F32(_) => number(c, i)
                    .and_then(serde_json::Number::from_f64)
                    .map_or(Value::Null, Value::Number),
            })
        };
        let rows = self.each(|id| nodes.json(id), cell, Value::from)?;
        Ok(rows.into_iter().map(Value::Array).collect())
    }

    /// The tab-separated form: a header, then every row shown, plain numbers (N-2).
    pub(crate) fn piped<S: ByteSource>(
        &self,
        nodes: &Nodes<'_, S>,
    ) -> Result<String, DictionaryError> {
        let cell = |_, c: &Column, i: usize| -> Result<String, DictionaryError> {
            Ok(match c {
                Column::Nodes { .. } => integer(c, i)
                    .map(|id| nodes.text(id))
                    .transpose()?
                    .unwrap_or_default(),
                Column::U32 { .. } => integer(c, i).map_or_else(String::new, |v| v.to_string()),
                Column::F64(v) => slice(v)[i].to_string(),
                Column::F32(v) => slice(v)[i].to_string(),
            })
        };
        let mut out = self.names().join("\t");
        out.push('\n');
        for row in self.each(|id| nodes.text(id), cell, |r| r.to_string())? {
            out.push_str(&row.join("\t"));
            out.push('\n');
        }
        Ok(out)
    }

    /// The terminal form: aligned with spaces, a dim header, numbers with separators,
    /// decimals to three significant digits of the smallest value shown, identifiers
    /// shortened in the middle only past 80 columns (W-4).
    pub(crate) fn aligned<S: ByteSource>(
        &self,
        nodes: &Nodes<'_, S>,
        style: Style,
    ) -> Result<String, DictionaryError> {
        let decimals: Vec<usize> = self.columns.iter().map(|(_, c)| self.decimals(c)).collect();
        let cell = |k: usize, c: &Column, i: usize| -> Result<String, DictionaryError> {
            Ok(match c {
                Column::Nodes { .. } => match integer(c, i) {
                    Some(id) => nodes.text(id)?,
                    None => "-".to_owned(),
                },
                Column::U32 { .. } => {
                    integer(c, i).map_or_else(|| "-".to_owned(), |v| count(u64::from(v)))
                }
                Column::F64(_) | Column::F32(_) => {
                    let x = number(c, i).unwrap_or(f64::NAN);
                    format!("{x:.d$}", d = decimals[k])
                }
            })
        };
        let mut cells = self.each(|id| nodes.text(id), cell, count)?;
        let names = self.names();
        let width = |cells: &[Vec<String>], k: usize| {
            cells
                .iter()
                .map(|r| r[k].chars().count())
                .chain([names[k].len()])
                .max()
                .unwrap_or(0)
        };
        let mut widths: Vec<usize> = (0..names.len()).map(|k| width(&cells, k)).collect();
        let total: usize = widths.iter().sum::<usize>() + 3 * (widths.len() - 1);
        if total > 80 {
            let room = widths[0].saturating_sub(total - 80).max(11);
            for row in &mut cells {
                row[0] = shorten(&row[0], room, style);
            }
            widths[0] = width(&cells, 0);
        }
        let line = |row: &[String]| {
            let mut l = format!("{:<w$}", row[0], w = widths[0]);
            for (k, cell) in row.iter().enumerate().skip(1) {
                let _ = write!(l, "   {cell:>w$}", w = widths[k]);
            }
            l
        };
        let header: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
        let mut out = format!("{}\n", style.dim(&line(&header)));
        for row in &cells {
            out.push_str(&line(row));
            out.push('\n');
        }
        Ok(out)
    }

    /// The decimals that show three significant digits of the smallest non-zero value
    /// of a floating-point column among the rows shown.
    fn decimals(&self, column: &Column) -> usize {
        let mut smallest = self
            .rows
            .iter()
            .filter_map(|&id| number(column, id as usize))
            .filter(|x| *x != 0.0 && x.is_finite())
            .map(f64::abs)
            .fold(f64::INFINITY, f64::min);
        if !smallest.is_finite() {
            return 2;
        }
        let mut digits = 0;
        while smallest < 100.0 && digits < 15 {
            smallest *= 10.0;
            digits += 1;
        }
        digits
    }
}

/// `text` shortened in the middle to `room` columns: its first six and last four
/// characters around `…` (W-4), or as many as fit.
fn shorten(text: &str, room: usize, style: Style) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= room {
        return text.to_owned();
    }
    let ellipsis = style.ellipsis();
    let keep = room.saturating_sub(ellipsis.chars().count());
    let tail = keep.min(4);
    let head = keep - tail;
    let head: String = chars[..head].iter().collect();
    let tail: String = chars[chars.len() - tail..].iter().collect();
    format!("{head}{ellipsis}{tail}")
}

/// Writes a table of columns `schema` to `path`, in the format of its extension, as
/// `PATH.partial` renamed into place once durable (ux-contract C-1): `fill` writes its
/// batches. Returns the bytes written.
///
/// # Errors
///
/// `UsageError` for an extension that names no format; `IoError`; what `fill` returns.
pub(crate) fn write(
    path: &Path,
    schema: &[(&str, Type)],
    fill: impl FnOnce(&mut TableWriter) -> Result<(), Failure>,
) -> Result<u64, Failure> {
    let format = format_of(path)?;
    let partial = PathBuf::from(format!("{}.partial", path.display()));
    let io = |e: &io::Error| failure::io("write", path, e);
    let file = File::create(&partial).map_err(|e| io(&e))?;
    let mut guard = Partial(Some(partial.clone()));
    let mut writer = TableWriter::create(file, format, schema).map_err(|e| io(&e))?;
    fill(&mut writer)?;
    let file = writer.finish().map_err(|e| io(&e))?;
    let bytes = file.metadata().map_or(0, |m| m.len());
    drop(file);
    everygraph_io::persist(&partial, path).map_err(|e| io(&e))?;
    guard.0 = None;
    Ok(bytes)
}

/// Writes the whole result (`node`, then `columns`, for every node) to `path`.
///
/// # Errors
///
/// As [`write`]; `CorruptFile` when an identifier cannot be read.
pub(crate) fn write_result<S: ByteSource>(
    path: &Path,
    graph: &Path,
    nodes: &Nodes<'_, S>,
    columns: &[(&'static str, Column)],
) -> Result<u64, Failure> {
    let rows = nodes.dictionary().len();
    let mut schema = vec![("node", nodes.file_type(false))];
    schema.extend(columns.iter().map(|(name, c)| {
        let ty = match c {
            Column::U32 { null, .. } => Type::U32(null.is_some()),
            Column::Nodes { .. } => nodes.file_type(true),
            Column::F64(_) => Type::F64,
            Column::F32(_) => Type::F32,
        };
        (*name, ty)
    }));
    let corrupt = |e: DictionaryError| failure::corrupt(graph, &e);
    write(path, &schema, |writer| {
        let mut start = 0_u32;
        loop {
            let end = start.saturating_add(BATCH).min(rows);
            let range = start as usize..end as usize;
            let ids = nodes.ids((start..end).map(Some)).map_err(corrupt)?;
            let mut owned = Vec::new();
            for (_, c) in columns {
                if let Column::Nodes { .. } = c {
                    owned.push(
                        nodes
                            .ids(range.clone().map(|i| integer(c, i)))
                            .map_err(corrupt)?,
                    );
                }
            }
            let mut owned = owned.iter();
            let mut batch = vec![ids.values()];
            for (_, c) in columns {
                batch.push(match c {
                    Column::U32 { values, null } => {
                        Values::U32(&slice(values)[range.clone()], *null)
                    }
                    Column::F64(v) => Values::F64(&slice(v)[range.clone()]),
                    Column::F32(v) => Values::F32(&slice(v)[range.clone()]),
                    Column::Nodes { .. } => owned.next().map_or(Values::F64(&[]), Ids::values),
                });
            }
            writer
                .write(&batch)
                .map_err(|e| failure::io("write", path, &e))?;
            if end == rows {
                return Ok(());
            }
            start = end;
        }
    })
}

/// The format of `path`'s extension.
///
/// # Errors
///
/// A usage error for an extension that names none.
pub(crate) fn format_of(path: &Path) -> Result<Format, Failure> {
    Format::of(path).ok_or_else(|| {
        let ending = path.extension().map_or_else(
            || "has no extension".to_owned(),
            |e| format!("ends in .{}", e.to_string_lossy()),
        );
        Failure::new(
            "UsageError",
            format!(
                "Cannot tell the format of {}: its name {ending}.",
                path.display()
            ),
            failure::USAGE,
        )
        .ways(vec![(
            format!("a name ending in {}", Format::EXTENSIONS),
            String::new(),
        )])
    })
}

/// Removes a `.partial` file unless it was renamed into place.
pub(crate) struct Partial(pub(crate) Option<PathBuf>);

impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_top_is_largest_first_ties_in_node_order_nulls_out() {
        let memory = everygraph_core::Memory::unlimited();
        let mut values = SegVec::try_with_len(&memory, "t", 7, 0_u32).unwrap();
        for (i, v) in [3, 9, u32::MAX, 9, 1, 3, 0].into_iter().enumerate() {
            values[i] = v;
        }
        let column = Column::U32 {
            values,
            null: Some(u32::MAX),
        };
        assert_eq!(top(&column, 7, 4), [1, 3, 0, 5]);
        assert_eq!(top(&column, 7, 100), [1, 3, 0, 5, 4, 6]);
        assert!(top(&column, 7, 0).is_empty());
    }

    #[test]
    fn identifiers_are_shortened_in_the_middle() {
        let style = Style {
            unicode: true,
            colour: false,
            terminal: true,
            piped: false,
        };
        let id = "0x28c6c06298d514db089934071355e5743bf21d60";
        assert_eq!(shorten(id, 11, style), "0x28c6…1d60");
        assert_eq!(shorten("short", 11, style), "short");
        let ascii = Style {
            unicode: false,
            ..style
        };
        assert_eq!(shorten(id, 11, ascii), "0x28...1d60");
    }
}
