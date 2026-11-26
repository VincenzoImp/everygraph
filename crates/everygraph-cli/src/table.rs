// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A result table — the `node` column, then one value per node — in its three forms
//! (`docs/api-sketch.md` § 5.3): aligned for a terminal, tab-separated when piped, and
//! the rows of the result document; and written whole to a file with `--out`. Columns
//! of nodes (`bfs`'s `parent`) show the nodes' identifiers.
//!
//! Rows are turned into text a batch at a time, under a reservation of the budget, so
//! a table of any length is printed or written in the memory of one batch.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use everygraph_core::{
    ByteSource, InsufficientBudget, Memory, MemoryKind, NodeId, Reservation, SegVec,
};
use everygraph_ingest::ids::{ColumnType, Dictionary, DictionaryError, KeyKind};
use everygraph_io::table::{Format, TableWriter, Type, Values};
use everygraph_kernels::run::Column;
use serde_json::Value;

use crate::failure::{self, Failure};
use crate::render::{Style, count};

/// Rows per batch.
const BATCH: usize = 1 << 14;
/// Bytes reserved per row of a batch: its identifiers and its text.
const ROW_BYTES: u64 = 256;
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

    /// Identifier `j`, as text; empty for none.
    fn text(&self, j: usize) -> String {
        match self {
            Self::Unsigned(v) => v[j].map_or_else(String::new, |x| x.to_string()),
            Self::Signed(v) => v[j].map_or_else(String::new, |x| x.to_string()),
            Self::Text(v) => v[j].clone().unwrap_or_default(),
        }
    }

    /// Identifier `j`, as JSON: a number for integers.
    fn json(&self, j: usize) -> Value {
        match self {
            Self::Unsigned(v) => v[j].map_or(Value::Null, Value::from),
            Self::Signed(v) => v[j].map_or(Value::Null, Value::from),
            Self::Text(v) => v[j].clone().map_or(Value::Null, Value::from),
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

    /// The file type of a column of identifiers.
    pub(crate) fn file_type(&self, nullable: bool) -> Type {
        match self.kind {
            Kind::Unsigned => Type::U64(nullable),
            Kind::Signed => Type::I64(nullable),
            Kind::Text => Type::Text(nullable),
        }
    }

    /// The identifiers of `ids`.
    pub(crate) fn ids(
        &self,
        ids: impl Iterator<Item = Option<u32>>,
    ) -> Result<Ids, DictionaryError> {
        let mut buf = Vec::new();
        let mut integer = |id: u32| -> Result<u64, DictionaryError> {
            buf.clear();
            self.dictionary.normalized_key(NodeId::new(id), &mut buf)?;
            let bytes: [u8; 8] = buf
                .as_slice()
                .try_into()
                .map_err(|_| DictionaryError::Corrupt("an integer key that is not 8 bytes"))?;
            Ok(u64::from_be_bytes(bytes))
        };
        Ok(match self.kind {
            Kind::Unsigned => Ids::Unsigned(
                ids.map(|id| id.map(&mut integer).transpose())
                    .collect::<Result<_, _>>()?,
            ),
            Kind::Signed => Ids::Signed(
                ids.map(|id| {
                    id.map(|id| Ok((integer(id)? ^ SIGN).cast_signed()))
                        .transpose()
                })
                .collect::<Result<_, DictionaryError>>()?,
            ),
            Kind::Text => Ids::Text(
                ids.map(|id| {
                    id.map(|id| Ok(self.dictionary.key(NodeId::new(id))?.to_string()))
                        .transpose()
                })
                .collect::<Result<_, DictionaryError>>()?,
            ),
        })
    }
}

/// The values of a column as a slice: one segment, which `run::run` and `run::stored`
/// make sure of.
fn slice<T: Copy>(v: &SegVec<T>) -> &[T] {
    v.as_slice().expect("a result column of one segment")
}

/// Value `i` of a column of integers or nodes, `None` for a null.
fn integer(column: &Column, i: usize) -> Option<u32> {
    match column {
        Column::U32 { values, null, .. } => Some(slice(values)[i]).filter(|v| Some(*v) != *null),
        Column::F64(_) | Column::F32(_) => None,
    }
}

/// Value `i` of a measure as `f64`, `None` for a null: the order of `--top`.
fn number(column: &Column, i: usize) -> Option<f64> {
    match column {
        Column::U32 { nodes: true, .. } => None,
        Column::U32 { .. } => integer(column, i).map(f64::from),
        Column::F64(v) => Some(slice(v)[i]),
        Column::F32(v) => Some(f64::from(slice(v)[i])),
    }
}

/// The rows a table shows.
pub(crate) enum Rows {
    /// The first rows, in graph order.
    First(usize),
    /// These rows, in this order.
    Chosen {
        ids: Vec<u32>,
        /// The budget they hold.
        _held: Reservation,
    },
}

impl Rows {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::First(n) => *n,
            Self::Chosen { ids, .. } => ids.len(),
        }
    }

    /// Rows `range` of this order.
    fn ids(&self, range: std::ops::Range<usize>) -> Vec<u32> {
        match self {
            Self::First(_) => (range.start..range.end)
                .map(|i| u32::try_from(i).expect("a node number"))
                .collect(),
            Self::Chosen { ids, .. } => ids[range].to_vec(),
        }
    }
}

/// The nodes of the largest `n` values of `column` among `rows` nodes, largest first,
/// ties in node order, nulls left out (`docs/api-sketch.md` § 2.12): a selection and a
/// sort of the `n`, in an array of the budget.
///
/// # Errors
///
/// The array of every node does not fit the budget.
pub(crate) fn top(
    column: &Column,
    rows: usize,
    n: usize,
    memory: &Memory,
) -> Result<Rows, InsufficientBudget> {
    let held = memory.reserve("the rows of --top", MemoryKind::Anonymous, rows as u64 * 4)?;
    let mut ids: Vec<u32> = (0..rows)
        .filter(|&i| number(column, i).is_some())
        .map(|i| u32::try_from(i).expect("a node number"))
        .collect();
    let value = |id: u32| number(column, id as usize).unwrap_or(f64::NEG_INFINITY);
    let order = |a: &u32, b: &u32| value(*b).total_cmp(&value(*a)).then(a.cmp(b));
    if n < ids.len() {
        ids.select_nth_unstable_by(n, order);
        ids.truncate(n);
    }
    ids.sort_unstable_by(order);
    Ok(Rows::Chosen { ids, _held: held })
}

/// The rows of one result to show, and whether they are ranked.
pub(crate) struct Shown<'r> {
    pub(crate) columns: &'r [(&'static str, Column)],
    pub(crate) rows: Rows,
    pub(crate) ranked: bool,
}

/// One batch of rows with their identifiers: of the rows, and of each column of nodes.
struct Batch {
    ids: Vec<u32>,
    nodes: Ids,
    columns: Vec<Option<Ids>>,
    /// The rank of the first row.
    first: u64,
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

    /// Calls `f` on the rows, a batch at a time, under a reservation of `memory`.
    fn batches<S: ByteSource>(
        &self,
        nodes: &Nodes<'_, S>,
        memory: &Memory,
        mut f: impl FnMut(&Batch) -> Result<(), Failure>,
        corrupt: &dyn Fn(DictionaryError) -> Failure,
    ) -> Result<(), Failure> {
        let _held = reserve(memory)?;
        let mut start = 0;
        while start < self.rows.len() {
            let end = (start + BATCH).min(self.rows.len());
            let ids = self.rows.ids(start..end);
            let columns = self
                .columns
                .iter()
                .map(|(_, c)| match c {
                    Column::U32 { nodes: true, .. } => nodes
                        .ids(ids.iter().map(|&i| integer(c, i as usize)))
                        .map(Some),
                    _ => Ok(None),
                })
                .collect::<Result<_, _>>()
                .map_err(corrupt)?;
            let batch = Batch {
                nodes: nodes.ids(ids.iter().copied().map(Some)).map_err(corrupt)?,
                columns,
                ids,
                first: start as u64 + 1,
            };
            f(&batch)?;
            start = end;
        }
        Ok(())
    }

    /// The text of row `j` of `batch`: identifiers, values (`human` for a terminal,
    /// plain otherwise, N-2), and the rank.
    fn cells(&self, batch: &Batch, j: usize, human: Option<&[usize]>) -> Vec<String> {
        let id = batch.ids[j] as usize;
        let mut row = vec![batch.nodes.text(j)];
        for (k, (_, c)) in self.columns.iter().enumerate() {
            let none = if human.is_some() { "-" } else { "" };
            row.push(
                match (c, &batch.columns[k], human) {
                    (_, Some(ids), _) => Some(ids.text(j)).filter(|t| !t.is_empty()),
                    (Column::U32 { .. }, None, Some(_)) => {
                        integer(c, id).map(|v| count(u64::from(v)))
                    }
                    (Column::U32 { .. }, None, None) => integer(c, id).map(|v| v.to_string()),
                    (Column::F64(v), None, None) => Some(slice(v)[id].to_string()),
                    (Column::F32(v), None, None) => Some(slice(v)[id].to_string()),
                    (_, None, Some(decimals)) => {
                        number(c, id).map(|x| format!("{x:.d$}", d = decimals[k]))
                    }
                }
                .unwrap_or_else(|| none.to_owned()),
            );
        }
        if self.ranked {
            let rank = batch.first + j as u64;
            row.push(if human.is_some() {
                count(rank)
            } else {
                rank.to_string()
            });
        }
        row
    }

    /// The rows, as the result document holds them.
    pub(crate) fn json_rows<S: ByteSource>(
        &self,
        nodes: &Nodes<'_, S>,
        memory: &Memory,
        corrupt: &dyn Fn(DictionaryError) -> Failure,
    ) -> Result<Vec<Value>, Failure> {
        let mut rows = Vec::new();
        self.batches(
            nodes,
            memory,
            |batch| {
                for j in 0..batch.ids.len() {
                    let id = batch.ids[j] as usize;
                    let mut row = vec![batch.nodes.json(j)];
                    for (k, (_, c)) in self.columns.iter().enumerate() {
                        row.push(match (&batch.columns[k], c) {
                            (Some(ids), _) => ids.json(j),
                            (None, Column::U32 { .. }) => {
                                integer(c, id).map_or(Value::Null, Value::from)
                            }
                            (None, _) => number(c, id)
                                .and_then(serde_json::Number::from_f64)
                                .map_or(Value::Null, Value::Number),
                        });
                    }
                    if self.ranked {
                        row.push((batch.first + j as u64).into());
                    }
                    rows.push(Value::Array(row));
                }
                Ok(())
            },
            corrupt,
        )?;
        Ok(rows)
    }

    /// The tab-separated form, written to `out` a batch at a time: a header, then every
    /// row shown, plain numbers (N-2).
    pub(crate) fn piped<S: ByteSource>(
        &self,
        nodes: &Nodes<'_, S>,
        memory: &Memory,
        out: &mut impl Write,
        corrupt: &dyn Fn(DictionaryError) -> Failure,
    ) -> Result<(), Failure> {
        let gone = |e: io::Error| failure::io("write", Path::new("standard output"), &e);
        writeln!(out, "{}", self.names().join("\t")).map_err(gone)?;
        let mut text = String::new();
        self.batches(
            nodes,
            memory,
            |batch| {
                text.clear();
                for j in 0..batch.ids.len() {
                    text.push_str(&self.cells(batch, j, None).join("\t"));
                    text.push('\n');
                }
                out.write_all(text.as_bytes()).map_err(gone)
            },
            corrupt,
        )?;
        out.flush().map_err(gone)
    }

    /// The terminal form: aligned with spaces, a dim header, numbers with separators,
    /// as many decimals as tell the values shown apart (at least three significant
    /// digits of the smallest), identifiers shortened in the middle only past 80
    /// columns (W-4).
    pub(crate) fn aligned<S: ByteSource>(
        &self,
        nodes: &Nodes<'_, S>,
        memory: &Memory,
        style: Style,
        corrupt: &dyn Fn(DictionaryError) -> Failure,
    ) -> Result<String, Failure> {
        let ids = self.rows.ids(0..self.rows.len());
        let decimals: Vec<usize> = self
            .columns
            .iter()
            .map(|(_, c)| decimals(c, &ids))
            .collect();
        let mut cells = Vec::new();
        self.batches(
            nodes,
            memory,
            |batch| {
                cells.extend((0..batch.ids.len()).map(|j| self.cells(batch, j, Some(&decimals))));
                Ok(())
            },
            corrupt,
        )?;
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
}

/// The budget of one batch of rows.
fn reserve(memory: &Memory) -> Result<Reservation, Failure> {
    memory
        .reserve(
            "a batch of rows",
            MemoryKind::Anonymous,
            BATCH as u64 * ROW_BYTES,
        )
        .map_err(|e| {
            failure::memory("Printing the rows", memory.available(), None, &e).on_existing_file()
        })
}

/// The decimals of a floating-point column among `ids`: three significant digits of
/// the smallest non-zero value, and more while two different values would print alike.
fn decimals(column: &Column, ids: &[u32]) -> usize {
    let values: Vec<f64> = ids
        .iter()
        .filter_map(|&id| number(column, id as usize))
        .filter(|x| x.is_finite())
        .collect();
    let mut smallest = values
        .iter()
        .filter(|x| **x != 0.0)
        .map(|x| x.abs())
        .fold(f64::INFINITY, f64::min);
    if !smallest.is_finite() {
        return 2;
    }
    let mut digits = 0;
    while smallest < 100.0 && digits < 15 {
        smallest *= 10.0;
        digits += 1;
    }
    let distinct: BTreeSet<u64> = values.iter().map(|x| x.to_bits()).collect();
    while digits < 17
        && values
            .iter()
            .map(|x| format!("{x:.digits$}"))
            .collect::<BTreeSet<_>>()
            .len()
            < distinct.len()
    {
        digits += 1;
    }
    digits
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

/// Writes the whole result (`node`, then `columns`, for every node) to `path`, a batch
/// at a time.
///
/// # Errors
///
/// As [`write()`]; `CorruptFile` when an identifier cannot be read.
pub(crate) fn write_result<S: ByteSource>(
    path: &Path,
    nodes: &Nodes<'_, S>,
    memory: &Memory,
    columns: &[(&'static str, Column)],
    corrupt: &dyn Fn(DictionaryError) -> Failure,
) -> Result<u64, Failure> {
    let mut schema = vec![("node", nodes.file_type(false))];
    schema.extend(columns.iter().map(|(name, c)| {
        let ty = match c {
            Column::U32 { nodes: true, .. } => nodes.file_type(true),
            Column::U32 { null, .. } => Type::U32(null.is_some()),
            Column::F64(_) => Type::F64,
            Column::F32(_) => Type::F32,
        };
        (*name, ty)
    }));
    let shown = Shown {
        columns,
        rows: Rows::First(nodes.dictionary().len() as usize),
        ranked: false,
    };
    write(path, &schema, |writer| {
        shown.batches(
            nodes,
            memory,
            |batch| {
                let range = batch.ids[0] as usize..batch.ids[0] as usize + batch.ids.len();
                let mut values = vec![batch.nodes.values()];
                for ((_, c), ids) in columns.iter().zip(&batch.columns) {
                    values.push(match (c, ids) {
                        (_, Some(ids)) => ids.values(),
                        (Column::U32 { values, null, .. }, None) => {
                            Values::U32(&slice(values)[range.clone()], *null)
                        }
                        (Column::F64(v), None) => Values::F64(&slice(v)[range.clone()]),
                        (Column::F32(v), None) => Values::F32(&slice(v)[range.clone()]),
                    });
                }
                writer
                    .write(&values)
                    .map_err(|e| failure::io("write", path, &e))
            },
            corrupt,
        )
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

    fn column(values: &[u32]) -> Column {
        let memory = Memory::unlimited();
        let mut v = SegVec::try_with_len(&memory, "t", values.len(), 0_u32).unwrap();
        for (i, x) in values.iter().enumerate() {
            v[i] = *x;
        }
        Column::U32 {
            values: v,
            null: Some(u32::MAX),
            nodes: false,
        }
    }

    fn chosen(rows: Rows) -> Vec<u32> {
        rows.ids(0..rows.len())
    }

    #[test]
    fn the_top_is_largest_first_ties_in_node_order_nulls_out() {
        let memory = Memory::unlimited();
        let c = column(&[3, 9, u32::MAX, 9, 1, 3, 0]);
        assert_eq!(chosen(top(&c, 7, 4, &memory).unwrap()), [1, 3, 0, 5]);
        assert_eq!(
            chosen(top(&c, 7, 100, &memory).unwrap()),
            [1, 3, 0, 5, 4, 6]
        );
        assert!(chosen(top(&c, 7, 0, &memory).unwrap()).is_empty());
        let small = Memory::from(std::sync::Arc::new(everygraph_core::Budget::new(8)));
        assert!(
            top(&c, 7, 1, &small).is_err(),
            "the array of every node is budgeted"
        );
    }

    #[test]
    fn the_top_agrees_with_a_full_sort() {
        let memory = Memory::unlimited();
        let mut x = 0x9E37_79B9_u32;
        let values: Vec<u32> = (0..5000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x % 50
            })
            .collect();
        let c = column(&values);
        let mut all: Vec<u32> = (0..5000).collect();
        all.sort_by(|a, b| values[*b as usize].cmp(&values[*a as usize]).then(a.cmp(b)));
        for n in [0, 1, 7, 49, 50, 51, 4999, 5000, 6000] {
            let got = chosen(top(&c, 5000, n, &memory).unwrap());
            assert_eq!(got, all[..n.min(5000)], "{n}");
        }
    }

    #[test]
    fn decimals_tell_the_values_shown_apart() {
        let memory = Memory::unlimited();
        let mut v = SegVec::try_with_len(&memory, "t", 3, 0.0_f64).unwrap();
        v[0] = 0.123_456_7;
        v[1] = 0.123_456_8;
        v[2] = 0.5;
        assert_eq!(decimals(&Column::F64(v), &[0, 1, 2]), 7);
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
