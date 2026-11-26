// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The Arrow IPC framing of a column section (`docs/spec/eg-columns.md` § 3): which
//! bytes go where, as a function of the column's [`Shape`] and its batches' null counts.
//! The writer emits it; the reader recomputes it and compares.

use std::ops::Range;

use super::flatbuf::{Field, Table, encode};
use super::{ColumnType, MAX_BATCH_ROWS, MAX_BODY_BYTES, MAX_NAME_BYTES};

/// Buffers start at multiples of this, counted from the start of the section (which is
/// itself a multiple of 4096 in the file).
pub(super) const PAGE: u64 = 4096;
/// The first 8 bytes of an Arrow IPC file: `ARROW1` and two zero bytes.
const FILE_MAGIC: &[u8; 8] = b"ARROW1\0\0";
const CONTINUATION: [u8; 4] = [0xFF; 4];
/// Arrow `MetadataVersion::V5`.
const V5: i16 = 4;
/// Arrow `MessageHeader` codes.
const SCHEMA: u8 = 1;
const DICTIONARY_BATCH: u8 = 2;
const RECORD_BATCH: u8 = 3;

/// What fixes a column section's bytes, besides its values and null counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Shape {
    pub(super) name: String,
    pub(super) ty: ColumnType,
    pub(super) nullable: bool,
    pub(super) rows: u64,
    pub(super) batch_rows: u64,
    /// For a dictionary-encoded column: the number of strings and their total bytes.
    pub(super) dictionary: Option<(u64, u64)>,
}

impl Shape {
    pub(super) fn batches(&self) -> u64 {
        self.rows.div_ceil(self.batch_rows)
    }

    /// Rows of batch `i`.
    pub(super) fn batch_len(&self, i: u64) -> u64 {
        self.batch_rows.min(self.rows - i * self.batch_rows)
    }

    /// Checks the limits of § 3.1 and § 2.
    pub(super) fn check(&self) -> Result<(), &'static str> {
        self.ty.check()?;
        if self.name.is_empty() || self.name.len() > MAX_NAME_BYTES {
            return Err("a column name is 1 to 512 bytes of UTF-8");
        }
        if !self.batch_rows.is_power_of_two() || self.batch_rows > MAX_BATCH_ROWS {
            return Err("the batch row count is a power of two up to 2^30");
        }
        let too_big = |lens: &[u64]| body(lens).is_none_or(|(_, len)| len >= MAX_BODY_BYTES);
        if too_big(&batch_lens(self.ty, self.batch_rows, self.batch_rows)) {
            return Err("a batch body would reach 2 GiB");
        }
        match (self.ty, self.dictionary) {
            (ColumnType::Dictionary(width), Some((values, bytes))) => {
                if values > width.most_values() {
                    return Err("more dictionary values than the codes can name");
                }
                // Bounded before `body` adds them up.
                if bytes >= MAX_BODY_BYTES || too_big(&dictionary_lens(values, bytes)) {
                    return Err("the dictionary body would reach 2 GiB");
                }
            }
            (ColumnType::Dictionary(_), None) | (_, Some(_)) => {
                return Err("a dictionary goes with a dictionary-encoded column, and only there");
            }
            _ => {}
        }
        Ok(())
    }
}

/// One message with a body: where it starts, its metadata (the continuation marker,
/// the length, the FlatBuffer; zero bytes follow up to the body), where its body starts,
/// its buffers, and where its body ends. Every byte from the end of `metadata` to `end`
/// that is not in a buffer is zero.
#[derive(Debug)]
pub(super) struct Frame {
    pub(super) at: u64,
    pub(super) metadata: Vec<u8>,
    pub(super) body: u64,
    pub(super) buffers: Vec<Range<u64>>,
    pub(super) end: u64,
}

/// The framing of one column section, message by message: [`Plan::new`] gives the
/// first bytes, then [`dictionary`](Plan::dictionary) (dictionary-encoded columns only)
/// and one [`batch`](Plan::batch) per batch, then [`tail`](Plan::tail).
#[derive(Debug)]
pub(super) struct Plan {
    shape: Shape,
    at: u64,
    dictionaries: Vec<u8>,
    batches: Vec<u8>,
}

impl Plan {
    /// The plan of a checked shape, and the bytes it starts with: the file magic and
    /// the schema message.
    pub(super) fn new(shape: Shape) -> (Self, Vec<u8>) {
        let mut head = FILE_MAGIC.to_vec();
        let fb = encode(&message(SCHEMA, schema(&shape), 0));
        let at = head.len() as u64;
        head.extend_from_slice(&prefix(
            at,
            fb.len(),
            (at + 8 + fb.len() as u64).next_multiple_of(8),
        ));
        head.extend_from_slice(&fb);
        head.resize(head.len().next_multiple_of(8), 0);
        let plan = Self {
            shape,
            at: head.len() as u64,
            dictionaries: Vec::new(),
            batches: Vec::new(),
        };
        (plan, head)
    }

    /// The dictionary batch: buffers validity (empty), offsets (`i32`), string bytes.
    pub(super) fn dictionary(&mut self) -> Frame {
        let (values, bytes) = self.shape.dictionary.unwrap_or((0, 0));
        let data = record_batch(values, 0, &dictionary_lens(values, bytes));
        let header = Table(vec![
            (0, Field::I64(0)),
            (1, Field::Table(data.0)),
            (2, Field::Bool(false)),
        ]);
        let frame = self.frame(DICTIONARY_BATCH, header, &data.1, data.2);
        push_block(&mut self.dictionaries, &frame);
        frame
    }

    /// The record batch of `rows` rows of which `nulls` are null: buffers validity
    /// (empty when `nulls` is 0) and values.
    pub(super) fn batch(&mut self, rows: u64, nulls: u64) -> Frame {
        let lens = batch_lens(self.shape.ty, rows, nulls);
        let (header, rel, body_len) = record_batch(rows, nulls, &lens);
        let frame = self.frame(RECORD_BATCH, header, &rel, body_len);
        push_block(&mut self.batches, &frame);
        frame
    }

    fn frame(&mut self, kind: u8, header: Table<'_>, rel: &[Range<u64>], body_len: u64) -> Frame {
        let fb = encode(&message(kind, header, body_len));
        let at = self.at;
        let start = (at + 8 + fb.len() as u64).next_multiple_of(PAGE);
        let mut metadata = prefix(at, fb.len(), start).to_vec();
        metadata.extend_from_slice(&fb);
        let buffers = rel.iter().map(|r| start + r.start..start + r.end).collect();
        self.at = start + body_len;
        Frame {
            at,
            metadata,
            body: start,
            buffers,
            end: self.at,
        }
    }

    /// The bytes after the last batch: the end-of-stream marker, the footer and its
    /// length, the file magic. The section ends with them.
    pub(super) fn tail(&self) -> Vec<u8> {
        let blocks = |bytes: &[u8]| Field::Structs(bytes.len() / 24, bytes.to_vec());
        let footer = encode(&Table(vec![
            (0, Field::I16(V5)),
            (1, Field::Table(schema(&self.shape))),
            (2, blocks(&self.dictionaries)),
            (3, blocks(&self.batches)),
        ]));
        let mut tail = CONTINUATION.to_vec();
        tail.extend_from_slice(&[0; 4]);
        tail.extend_from_slice(&footer);
        tail.extend_from_slice(&i32_of(footer.len() as u64).to_le_bytes());
        tail.extend_from_slice(b"ARROW1");
        tail
    }

    /// Where the next message starts.
    pub(super) fn position(&self) -> u64 {
        self.at
    }

    pub(super) fn shape(&self) -> &Shape {
        &self.shape
    }
}

/// The continuation marker and the metadata length of a message at `at` whose
/// FlatBuffer has `fb_len` bytes and whose body starts at `body`.
fn prefix(at: u64, fb_len: usize, body: u64) -> [u8; 8] {
    debug_assert!(body >= at + 8 + fb_len as u64);
    let mut p = [0; 8];
    p[..4].copy_from_slice(&CONTINUATION);
    p[4..].copy_from_slice(&i32_of(body - at - 8).to_le_bytes());
    p
}

fn i32_of(x: u64) -> i32 {
    i32::try_from(x).unwrap_or(i32::MAX)
}

fn i64_of(x: u64) -> i64 {
    i64::try_from(x).unwrap_or(i64::MAX)
}

/// An Arrow `Block` (offset, metadata length, body length) for the footer.
fn push_block(out: &mut Vec<u8>, frame: &Frame) {
    out.extend_from_slice(&i64_of(frame.at).to_le_bytes());
    out.extend_from_slice(&i32_of(frame.body - frame.at).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&i64_of(frame.end - frame.body).to_le_bytes());
}

fn message<'a>(kind: u8, header: Table<'a>, body_len: u64) -> Table<'a> {
    Table(vec![
        (0, Field::I16(V5)),
        (1, Field::U8(kind)),
        (2, Field::Table(header)),
        (3, Field::I64(i64_of(body_len))),
    ])
}

/// A `RecordBatch` of one field, with its buffers placed in the body; also returns
/// the buffers' ranges relative to the body and the body's length.
fn record_batch(rows: u64, nulls: u64, lens: &[u64]) -> (Table<'static>, Vec<Range<u64>>, u64) {
    let (rel, body_len) = body(lens).expect("the lengths of a checked shape (`Shape::check`)");
    let mut node = i64_of(rows).to_le_bytes().to_vec();
    node.extend_from_slice(&i64_of(nulls).to_le_bytes());
    let mut buffers = Vec::with_capacity(16 * rel.len());
    for r in &rel {
        buffers.extend_from_slice(&i64_of(r.start).to_le_bytes());
        buffers.extend_from_slice(&i64_of(r.end - r.start).to_le_bytes());
    }
    let table = Table(vec![
        (0, Field::I64(i64_of(rows))),
        (1, Field::Structs(1, node)),
        (2, Field::Structs(rel.len(), buffers)),
    ]);
    (table, rel, body_len)
}

/// Places buffers of these lengths in a body: an empty one at offset 0, every other at
/// the first multiple of [`PAGE`] after the previous non-empty one; the body ends at the
/// next multiple of 8 after the last. `None` past `u64`.
pub(super) fn body(lens: &[u64]) -> Option<(Vec<Range<u64>>, u64)> {
    let mut end = 0u64;
    let mut ranges = Vec::with_capacity(lens.len());
    for &len in lens {
        if len == 0 {
            ranges.push(0..0);
            continue;
        }
        let start = end.checked_next_multiple_of(PAGE)?;
        end = start.checked_add(len)?;
        ranges.push(start..end);
    }
    Some((ranges, end.checked_next_multiple_of(8)?))
}

/// Buffer lengths of a batch: validity (empty without nulls), values.
pub(super) fn batch_lens(ty: ColumnType, rows: u64, nulls: u64) -> [u64; 2] {
    let validity = if nulls > 0 { rows.div_ceil(8) } else { 0 };
    let values = match ty {
        ColumnType::Bool => rows.div_ceil(8),
        _ => rows.saturating_mul(ty.width()),
    };
    [validity, values]
}

/// Buffer lengths of the dictionary batch: validity (empty), offsets, string bytes.
fn dictionary_lens(values: u64, bytes: u64) -> [u64; 3] {
    [0, values.saturating_add(1).saturating_mul(4), bytes]
}

fn schema(shape: &Shape) -> Table<'_> {
    let (code, ty) = arrow_type(shape.ty);
    let mut field = vec![
        (0, Field::Str(&shape.name)),
        (1, Field::Bool(shape.nullable)),
        (2, Field::U8(code)),
        (3, Field::Table(ty)),
    ];
    if let ColumnType::Dictionary(width) = shape.ty {
        let bits = i32::from(width.bits());
        field.push((
            4,
            Field::Table(Table(vec![
                (0, Field::I64(0)),
                (1, Field::Table(int(bits, false))),
                (2, Field::Bool(false)),
                (3, Field::I16(0)),
            ])),
        ));
    }
    field.push((5, Field::Tables(Vec::new())));
    Table(vec![
        (0, Field::I16(0)),
        (1, Field::Tables(vec![Table(field)])),
    ])
}

fn int(bits: i32, signed: bool) -> Table<'static> {
    Table(vec![(0, Field::I32(bits)), (1, Field::Bool(signed))])
}

/// The Arrow `Type` union member (its code and its table) of a column type; the value
/// type for a dictionary-encoded column.
fn arrow_type(ty: ColumnType) -> (u8, Table<'static>) {
    let decimal = |precision: u8, scale: i8, bits| {
        Table(vec![
            (0, Field::I32(precision.into())),
            (1, Field::I32(scale.into())),
            (2, Field::I32(bits)),
        ])
    };
    match ty {
        ColumnType::Int8 => (2, int(8, true)),
        ColumnType::Int16 => (2, int(16, true)),
        ColumnType::Int32 => (2, int(32, true)),
        ColumnType::Int64 => (2, int(64, true)),
        ColumnType::UInt8 => (2, int(8, false)),
        ColumnType::UInt16 => (2, int(16, false)),
        ColumnType::UInt32 => (2, int(32, false)),
        ColumnType::UInt64 => (2, int(64, false)),
        ColumnType::Float32 => (3, Table(vec![(0, Field::I16(1))])),
        ColumnType::Float64 => (3, Table(vec![(0, Field::I16(2))])),
        ColumnType::Dictionary(_) => (5, Table(Vec::new())),
        ColumnType::Bool => (6, Table(Vec::new())),
        ColumnType::Decimal128 { precision, scale } => (7, decimal(precision, scale, 128)),
        ColumnType::Decimal256 { precision, scale } => (7, decimal(precision, scale, 256)),
        ColumnType::Timestamp(unit) => (
            10,
            Table(vec![
                (0, Field::I16(unit.code().into())),
                (1, Field::Str("UTC")),
            ]),
        ),
        ColumnType::FixedSizeBinary(width) => {
            (15, Table(vec![(0, Field::I32(i32_of(u64::from(width))))]))
        }
    }
}
