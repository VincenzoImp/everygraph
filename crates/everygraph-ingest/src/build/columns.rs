// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The edge columns through the build (`docs/spec/eg-columns.md` § 6): what an item
//! carries, how a pair's events become its stored values, and the columns written
//! from them.
//!
//! An item is a target ([`u32`]) when the graph has no edge column, or a [`Tagged`]
//! target that also carries its row's position, weight and time. Both go through the
//! same passes; only the bytes per item and the sort key differ. A list is sorted by
//! (target, time, input order) under `duplicates="events"` and by (target, input
//! order) otherwise, so a pair's events come in their stored order and the first of
//! them is the row `collapse` keeps (`docs/spec/graph-semantics.md` § 2.3, § 2.5).

use std::io::{self, BufWriter, Write};

use everygraph_core::math::canonical;
use everygraph_core::{ByteSink, ExactSum, Memory, MemoryKind, Reservation, widen};
use everygraph_format::columns::{ColumnSpec, ColumnType, ColumnWriter, TimeUnit, Values};
use everygraph_format::container::Writer;

use super::fill::Chunked;
use super::{BuildError, Duplicates};
use crate::ids::Scratch;
use crate::text::{Batch, Layout as TextLayout};

/// What the lists of the build are made of.
pub(super) trait Item: Copy + Default + Send + Sync + 'static {
    /// The order of a node's items: target first.
    type Key: Ord;
    /// Bytes of a spill record of the item.
    const BYTES: usize;

    fn target(self) -> u32;
    fn key(self, by_time: bool) -> Self::Key;
    fn write(self, out: &mut [u8]);
    fn read(bytes: &[u8]) -> Self;
    /// Appends what the events `group` of one pair store to `values`, one vector per
    /// column of `columns`.
    fn pair(group: &[Self], columns: &[Out], values: &mut [Vec<u64>]);
}

/// An item made from an input row (module `rows`).
pub(super) trait FromRow: Item {
    /// What a row gives each of its items besides the target.
    type Payload: Copy + Default + Send + Sync + 'static;

    fn new(target: u32, payload: Self::Payload) -> Self;
    fn payload(batch: &Batch, block: usize, row: usize) -> Self::Payload;
}

impl FromRow for u32 {
    type Payload = ();

    fn new(target: u32, (): ()) -> Self {
        target
    }
    fn payload(_: &Batch, _: usize, _: usize) {}
}

impl Item for u32 {
    type Key = u32;
    const BYTES: usize = 4;

    fn target(self) -> u32 {
        self
    }
    fn key(self, _: bool) -> u32 {
        self
    }
    fn write(self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_le_bytes());
    }
    fn read(bytes: &[u8]) -> Self {
        Self::from_le_bytes(bytes.try_into().expect("4 bytes"))
    }
    fn pair(group: &[Self], columns: &[Out], values: &mut [Vec<u64>]) {
        // Without a weight or a time, the one column is `edges` (`duplicates="sum"`).
        for (column, out) in columns.iter().zip(values) {
            debug_assert_eq!(column.what, What::Edges);
            out.push(widen(group.len()));
        }
    }
}

/// The row of an item: its position in the input (`block << 32 | row`, which increases
/// along the input), its weight and its time (0 for a column the graph does not have).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Row {
    pub(super) at: u64,
    pub(super) weight: f64,
    pub(super) time: i64,
}

/// A target with its row: 28 bytes, as in a spill record, without the padding that
/// aligning the row to 8 bytes would add. Fields are only read and written by value.
#[derive(Clone, Copy, Default)]
#[repr(C, packed(4))]
pub(super) struct Tagged {
    target: u32,
    row: Row,
}

const _: () = assert!(size_of::<Tagged>() == Tagged::BYTES);

impl FromRow for Tagged {
    type Payload = Row;

    fn new(target: u32, row: Row) -> Self {
        Self { target, row }
    }
    fn payload(batch: &Batch, block: usize, row: usize) -> Row {
        Row {
            at: widen(block) << 32 | widen(row),
            weight: batch.weight.as_ref().map_or(0.0, |w| w[row]),
            time: batch.time.as_ref().map_or(0, |t| t[row]),
        }
    }
}

impl Item for Tagged {
    type Key = (u32, i64, u64);
    const BYTES: usize = 4 + 8 + 8 + 8;

    fn target(self) -> u32 {
        self.target
    }
    fn key(self, by_time: bool) -> Self::Key {
        let time = if by_time { self.row.time } else { 0 };
        (self.target, time, self.row.at)
    }
    fn write(self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.target.to_le_bytes());
        out[4..12].copy_from_slice(&self.row.at.to_le_bytes());
        out[12..20].copy_from_slice(&self.row.weight.to_bits().to_le_bytes());
        out[20..28].copy_from_slice(&self.row.time.to_le_bytes());
    }
    fn read(b: &[u8]) -> Self {
        let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"));
        Self {
            target: u32::from_le_bytes(b[..4].try_into().expect("4 bytes")),
            row: Row {
                at: u64_at(4),
                weight: f64::from_bits(u64_at(12)),
                time: i64::from_le_bytes(u64_at(20).to_le_bytes()),
            },
        }
    }
    fn pair(group: &[Self], columns: &[Out], values: &mut [Vec<u64>]) {
        let weights = || group.iter().map(|e| e.row.weight);
        let times = || group.iter().map(|e| e.row.time);
        for (column, out) in columns.iter().zip(values) {
            match column.what {
                What::EventWeight => out.extend(weights().map(f64::to_bits)),
                What::EventTime => out.extend(times().map(bits_of)),
                What::PairSum => out.push(pair_sum(group).to_bits()),
                What::Kept => out.push(group[0].row.weight.to_bits()),
                What::KeptTime => out.push(bits_of(group[0].row.time)),
                What::Edges => out.push(widen(group.len())),
                What::First => out.push(bits_of(times().min().unwrap_or(0))),
                What::Last => out.push(bits_of(times().max().unwrap_or(0))),
            }
        }
    }
}

/// An item of the in-adjacency (module `transpose`): the target, the offset of the
/// out-adjacency's event it reverses, and the values the out-adjacency stores for that
/// event, one per column of its `N` (a per-pair value is that of the event's entry). A
/// list is sorted by (target, offset): the events of a pair keep the out-adjacency's
/// order, and the values are copied, never computed again.
#[derive(Clone, Copy)]
#[repr(C, packed(4))]
pub(super) struct Copied<const N: usize> {
    pub(super) target: u32,
    pub(super) at: u64,
    pub(super) values: [u64; N],
}

impl<const N: usize> Default for Copied<N> {
    fn default() -> Self {
        Self {
            target: 0,
            at: 0,
            values: [0; N],
        }
    }
}

impl<const N: usize> Item for Copied<N> {
    type Key = (u32, u64);
    const BYTES: usize = {
        assert!(size_of::<Self>() == 12 + 8 * N);
        12 + 8 * N
    };

    fn target(self) -> u32 {
        self.target
    }
    fn key(self, _: bool) -> Self::Key {
        (self.target, self.at)
    }
    fn write(self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.target.to_le_bytes());
        out[4..12].copy_from_slice(&self.at.to_le_bytes());
        let values = self.values;
        for (k, v) in values.iter().enumerate() {
            out[12 + 8 * k..20 + 8 * k].copy_from_slice(&v.to_le_bytes());
        }
    }
    fn read(b: &[u8]) -> Self {
        let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"));
        Self {
            target: u32::from_le_bytes(b[..4].try_into().expect("4 bytes")),
            at: u64_at(4),
            values: std::array::from_fn(|k| u64_at(12 + 8 * k)),
        }
    }
    fn pair(group: &[Self], columns: &[Out], values: &mut [Vec<u64>]) {
        for (k, (column, out)) in columns.iter().zip(values).enumerate() {
            if column.per_event() {
                out.extend(group.iter().map(|e| e.values[k]));
            } else {
                out.push(group[0].values[k]);
            }
        }
    }
}

fn bits_of(time: i64) -> u64 {
    u64::from_le_bytes(time.to_le_bytes())
}

/// The exact sum of the weights of a pair's events, rounded once
/// (`docs/spec/graph-semantics.md` § 4.3).
fn pair_sum(group: &[Tagged]) -> f64 {
    match group {
        [one] => canonical(one.row.weight),
        _ => group
            .iter()
            .map(|e| e.row.weight)
            .collect::<ExactSum>()
            .value(),
    }
}

/// What a stored column holds, per event or per pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum What {
    /// Each event's weight (`"events"`).
    EventWeight,
    /// Each event's time (`"events"`).
    EventTime,
    /// The pair sum of the weights (`"events"`, `"sum"`).
    PairSum,
    /// The kept row's weight (`"collapse"`).
    Kept,
    /// The kept row's time (`"collapse"`).
    KeptTime,
    /// The rows merged into the pair (`"sum"`).
    Edges,
    /// The earliest time of the pair's rows (`"sum"`).
    First,
    /// The latest (`"sum"`).
    Last,
}

/// One column the build stores for an adjacency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Out {
    what: What,
    pub(super) name: String,
}

impl Out {
    pub(super) fn per_event(&self) -> bool {
        matches!(self.what, What::EventWeight | What::EventTime)
    }

    fn ty(&self) -> ColumnType {
        match self.what {
            What::EventWeight | What::PairSum | What::Kept => ColumnType::Float64,
            What::Edges => ColumnType::UInt64,
            _ => ColumnType::Timestamp(TimeUnit::Microsecond),
        }
    }
}

/// The columns stored for the weight and time columns of `layout` under `duplicates`,
/// and the `edges` of `duplicates="sum"` (`docs/spec/eg-columns.md` § 6;
/// `docs/spec/graph-semantics.md` § 2.4). Text gives `float64` weights and times in
/// microseconds, never null.
pub(super) fn outputs(layout: &TextLayout, duplicates: Duplicates) -> Vec<Out> {
    let name = |c: Option<usize>| c.map(|c| layout.column_name(c));
    let (weight, time) = (name(layout.weight), name(layout.time));
    outputs_of(weight.as_deref(), time.as_deref(), duplicates)
}

/// The columns of [`outputs`] for a weight and a time column of these names.
pub(super) fn outputs_of(
    weight: Option<&str>,
    time: Option<&str>,
    duplicates: Duplicates,
) -> Vec<Out> {
    let out = |what, name: &str| Out {
        what,
        name: name.to_owned(),
    };
    let mut columns = Vec::new();
    match duplicates {
        Duplicates::Events => {
            if let Some(w) = &weight {
                columns.extend([out(What::EventWeight, w), out(What::PairSum, w)]);
            }
            if let Some(t) = &time {
                columns.push(out(What::EventTime, t));
            }
        }
        Duplicates::Collapse => {
            columns.extend(weight.iter().map(|w| out(What::Kept, w)));
            columns.extend(time.iter().map(|t| out(What::KeptTime, t)));
        }
        Duplicates::Sum => {
            if let Some(w) = &weight {
                columns.push(out(What::PairSum, w));
            }
            columns.push(out(What::Edges, "edges"));
            if time.is_some() {
                columns.extend([out(What::First, "first"), out(What::Last, "last")]);
            }
        }
    }
    columns
}

/// A name that two of `columns` would be stored under in the same scope: a weight
/// column named `edges`, `first` or `last` under `duplicates="sum"`, or one column read
/// as both the weight and the time.
pub(super) fn clash(columns: &[Out]) -> Option<&str> {
    columns.iter().enumerate().find_map(|(i, a)| {
        let same = |b: &Out| b.name == a.name && b.per_event() == a.per_event();
        columns[..i].iter().any(same).then_some(a.name.as_str())
    })
}

/// The values of each column of [`outputs`], kept in a scratch file per column as
/// they come, bucket by bucket, as little-endian `u64` (the bits of an `f64`, of an
/// `i64`, or a `u64`).
pub(super) struct Stored<W: Write> {
    files: Vec<(BufWriter<W>, u64)>,
    _buffers: Reservation,
}

/// Write buffer in front of each column file, and read buffer when it is copied.
pub(super) const COLUMN_BUFFER: usize = 64 << 10;

impl<W: Write> Stored<W> {
    pub(super) fn new<Sc: Scratch<Writer = W>>(
        columns: &[Out],
        scratch: &Sc,
        memory: &Memory,
    ) -> Result<Self, BuildError> {
        let bytes = widen(columns.len() * COLUMN_BUFFER);
        let buffers = memory.reserve("column buffers", MemoryKind::Anonymous, bytes)?;
        let files = columns
            .iter()
            .map(|_| {
                Ok((
                    BufWriter::with_capacity(COLUMN_BUFFER, scratch.create("column")?),
                    0,
                ))
            })
            .collect::<io::Result<_>>()?;
        Ok(Self {
            files,
            _buffers: buffers,
        })
    }

    /// Appends one bucket's values: one vector per column.
    pub(super) fn push(&mut self, values: &[Vec<u64>]) -> io::Result<()> {
        for ((file, rows), column) in self.files.iter_mut().zip(values) {
            for &x in column {
                file.write_all(&x.to_le_bytes())?;
            }
            *rows += widen(column.len());
        }
        Ok(())
    }

    /// Writes every column into `writer`, under the scope `<adjacency>.event` or
    /// `<adjacency>.pair`. Each must have `events` or `entries` rows.
    pub(super) fn write<Sc: Scratch<Writer = W>, K: ByteSink>(
        self,
        columns: &[Out],
        adjacency: &str,
        (entries, events): (u64, u64),
        scratch: &Sc,
        writer: &mut Writer<K>,
        memory: &Memory,
    ) -> Result<(), BuildError> {
        let mut buf = Decoded::default();
        for ((file, rows), column) in self.files.into_iter().zip(columns) {
            let (scope, expected) = if column.per_event() {
                (format!("{adjacency}.event"), events)
            } else {
                (format!("{adjacency}.pair"), entries)
            };
            debug_assert_eq!(rows, expected, "{}", column.name);
            let file = file.into_inner().map_err(io::IntoInnerError::into_error)?;
            let source = scratch.seal(file)?;
            let spec = ColumnSpec::new(&column.name, column.ty(), rows);
            let mut out = ColumnWriter::begin(writer, &scope, &spec, None)?;
            let mut reader = Chunked::new(&source, memory)?;
            let mut left = rows;
            while left > 0 {
                let mut batch = left.min(spec.batch_rows);
                left -= batch;
                out.begin_batch()?;
                while batch > 0 {
                    // `Chunked` hands out at most 4 KiB at a time.
                    let n = batch.min(512);
                    batch -= n;
                    let bytes = reader
                        .take(usize::try_from(n * 8).expect("small"))?
                        .ok_or_else(|| io::Error::other("a column file ends early"))?;
                    push(&mut out, column.ty(), bytes, &mut buf)?;
                }
                out.end_batch()?;
            }
            out.finish()?;
            drop(source);
        }
        Ok(())
    }
}

/// Hands `bytes`, little-endian `u64`, to `out` as values of `ty`.
fn push<K: ByteSink>(
    out: &mut ColumnWriter<'_, K>,
    ty: ColumnType,
    bytes: &[u8],
    buf: &mut Decoded,
) -> Result<(), BuildError> {
    let words = bytes
        .chunks_exact(8)
        .map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")));
    match ty {
        ColumnType::Float64 => {
            buf.f64s.clear();
            buf.f64s.extend(words.map(f64::from_bits));
            out.push(Values::F64(&buf.f64s))?;
        }
        ColumnType::UInt64 => {
            buf.u64s.clear();
            buf.u64s.extend(words);
            out.push(Values::U64(&buf.u64s))?;
        }
        _ => {
            buf.i64s.clear();
            buf.i64s
                .extend(words.map(|w| i64::from_le_bytes(w.to_le_bytes())));
            out.push(Values::I64(&buf.i64s))?;
        }
    }
    Ok(())
}

/// The buffer [`push`] decodes a piece of a column file into, one per type.
#[derive(Default)]
struct Decoded {
    f64s: Vec<f64>,
    u64s: Vec<u64>,
    i64s: Vec<i64>,
}
