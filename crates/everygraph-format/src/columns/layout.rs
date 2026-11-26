// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The layout section of a column (`docs/spec/eg-columns.md` § 4): its type and shape,
//! and where every buffer of its Arrow file lies.

use std::ops::Range;

use super::arrow::{Plan, Shape};
use super::{CodeWidth, ColumnError, ColumnType, MAX_LAYOUT_BYTES, MAX_NAME_BYTES, TimeUnit};

const MAGIC: &[u8; 8] = b"EGCOLLAY";
const VERSION: u32 = 1;
const HEADER: usize = 80;
const DICTIONARY: usize = 32;
const BATCH: usize = 40;
const DICTIONARY_CODE: u8 = 8;

/// Where one batch's buffers lie, in bytes from the start of the column section (and so
/// at multiples of 4096 in the file, as sections are).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BatchLayout {
    /// Rows of the batch that are null.
    pub null_count: u64,
    /// The validity bitmap (bit `i` of byte `i / 8` set when row `i` is not null); empty
    /// when the batch has no null.
    pub validity: Range<u64>,
    /// The values: little-endian, `width` bytes per row, or one bit per row for `bool`.
    pub values: Range<u64>,
}

/// Where the strings of a dictionary-encoded column lie.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DictionaryLayout {
    /// The number of strings; codes are below it.
    pub values: u64,
    /// `values + 1` little-endian `i32`: where each string starts in `data`, then its end.
    pub offsets: Range<u64>,
    /// The strings' UTF-8 bytes, concatenated.
    pub data: Range<u64>,
}

/// A parsed layout section: what the column holds and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnLayout {
    shape: Shape,
    column_len: u64,
    dictionary: Option<DictionaryLayout>,
    batches: Vec<BatchLayout>,
}

impl ColumnLayout {
    /// The layout of a column of this shape whose batches have these null counts.
    pub(super) fn new(shape: Shape, nulls: &[u64]) -> Result<Self, ColumnError> {
        check(&shape)?;
        if nulls.len() as u64 != shape.batches() {
            return Err(ColumnError::Input("one null count per batch"));
        }
        let (mut plan, _) = Plan::new(shape.clone());
        let dictionary = shape.dictionary.map(|(values, _)| {
            let mut frame = plan.dictionary();
            let data = frame.buffers.pop().unwrap_or_default();
            let offsets = frame.buffers.pop().unwrap_or_default();
            DictionaryLayout {
                values,
                offsets,
                data,
            }
        });
        let mut batches = Vec::with_capacity(nulls.len());
        for (i, &null_count) in (0..).zip(nulls) {
            let rows = shape.batch_len(i);
            if null_count > rows || (null_count > 0 && !shape.nullable) {
                return Err(ColumnError::Input(
                    "a null count above the batch's rows, or nulls in a column that is not nullable",
                ));
            }
            let mut frame = plan.batch(rows, null_count);
            let values = frame.buffers.pop().unwrap_or_default();
            let validity = frame.buffers.pop().unwrap_or_default();
            batches.push(BatchLayout {
                null_count,
                validity,
                values,
            });
        }
        let column_len = plan.position() + plan.tail().len() as u64;
        Ok(Self {
            shape,
            column_len,
            dictionary,
            batches,
        })
    }

    /// Parses a layout section. The bytes must be exactly those a writer writes for the
    /// shape and null counts they record (§ 4): every offset is checked that way.
    ///
    /// # Errors
    ///
    /// [`ColumnError::Unsupported`] for a version or flags this reader does not know;
    /// [`ColumnError::Invalid`] for anything else that breaks the specification;
    /// [`ColumnError::TooLarge`] above [`MAX_LAYOUT_BYTES`].
    pub fn parse(bytes: &[u8]) -> Result<Self, ColumnError> {
        if bytes.len() as u64 > MAX_LAYOUT_BYTES {
            return Err(ColumnError::TooLarge("a layout section exceeds 16 MiB"));
        }
        let header = bytes
            .get(..HEADER)
            .ok_or(ColumnError::Invalid("a layout shorter than its header"))?;
        if &header[..8] != MAGIC {
            return Err(ColumnError::Invalid("not a column layout (bad magic)"));
        }
        let u32_at =
            |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap_or_default());
        let u64_at =
            |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().unwrap_or_default());
        if u32_at(8) != VERSION {
            return Err(ColumnError::Unsupported(
                "a column layout version other than 1",
            ));
        }
        if u32_at(12) != 0 {
            return Err(ColumnError::Unsupported("column layout flags"));
        }
        let invalid = ColumnError::Invalid;
        let (rows, batch_rows, batches) = (u64_at(16), u64_at(24), u64_at(32));
        let ty = decode_type(header[48], u32_at(52), u32_at(56)).ok_or(
            ColumnError::Unsupported("a column type this reader does not know"),
        )?;
        let nullable = match header[49] {
            0 => false,
            1 => true,
            _ => return Err(invalid("the nullable byte is 0 or 1")),
        };
        let name_len = u32_at(60) as usize;
        if name_len > MAX_NAME_BYTES {
            return Err(invalid("a column name longer than 512 bytes"));
        }
        if !batch_rows.is_power_of_two() || batches != rows.div_ceil(batch_rows) {
            return Err(invalid("the batch count does not match the rows"));
        }
        let dictionary = (header[48] == DICTIONARY_CODE).then(|| (u64_at(64), u64_at(72)));
        let name_end = HEADER + name_len.next_multiple_of(8);
        let first = name_end + if dictionary.is_some() { DICTIONARY } else { 0 };
        let expected = usize::try_from(batches)
            .ok()
            .and_then(|b| b.checked_mul(BATCH)?.checked_add(first));
        if expected != Some(bytes.len()) {
            return Err(invalid("the layout's length does not match its header"));
        }
        let name = std::str::from_utf8(&bytes[HEADER..HEADER + name_len])
            .map_err(|_| invalid("the column name is not UTF-8"))?;
        let nulls: Vec<u64> = bytes[first..]
            .chunks_exact(BATCH)
            .map(|b| u64::from_le_bytes(b[..8].try_into().unwrap_or_default()))
            .collect();
        let shape = Shape {
            name: name.to_owned(),
            ty,
            nullable,
            rows,
            batch_rows,
            dictionary,
        };
        let layout = Self::new(shape, &nulls).map_err(|e| match e {
            ColumnError::Input(what) => invalid(what),
            other => other,
        })?;
        if layout.encode() != bytes {
            return Err(invalid("the layout is not the one its shape implies"));
        }
        Ok(layout)
    }

    /// The bytes of the layout section.
    pub(super) fn encode(&self) -> Vec<u8> {
        let s = &self.shape;
        let (code, p1, p2) = encode_type(s.ty);
        let (values, bytes) = s.dictionary.unwrap_or((0, 0));
        let mut out =
            Vec::with_capacity(layout_len(s).map_or(0, |n| usize::try_from(n).unwrap_or(0)));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for x in [s.rows, s.batch_rows, s.batches(), self.column_len] {
            out.extend_from_slice(&x.to_le_bytes());
        }
        out.extend_from_slice(&[code, u8::from(s.nullable), 0, 0]);
        out.extend_from_slice(&p1.to_le_bytes());
        out.extend_from_slice(&p2.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(s.name.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        out.extend_from_slice(&values.to_le_bytes());
        out.extend_from_slice(&bytes.to_le_bytes());
        out.extend_from_slice(s.name.as_bytes());
        out.resize(out.len().next_multiple_of(8), 0);
        let mut range = |r: &Range<u64>| {
            out.extend_from_slice(&r.start.to_le_bytes());
            out.extend_from_slice(&(r.end - r.start).to_le_bytes());
        };
        if let Some(d) = &self.dictionary {
            range(&d.offsets);
            range(&d.data);
        }
        for b in &self.batches {
            out.extend_from_slice(&b.null_count.to_le_bytes());
            out.extend_from_slice(&b.validity.start.to_le_bytes());
            out.extend_from_slice(&(b.validity.end - b.validity.start).to_le_bytes());
            out.extend_from_slice(&b.values.start.to_le_bytes());
            out.extend_from_slice(&(b.values.end - b.values.start).to_le_bytes());
        }
        out
    }

    pub(super) fn shape(&self) -> &Shape {
        &self.shape
    }

    /// The column's name, which is also its Arrow field name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.shape.name
    }

    /// The column's type.
    #[must_use]
    pub fn ty(&self) -> ColumnType {
        self.shape.ty
    }

    /// Whether the column may hold nulls.
    #[must_use]
    pub fn nullable(&self) -> bool {
        self.shape.nullable
    }

    /// The number of rows.
    #[must_use]
    pub fn rows(&self) -> u64 {
        self.shape.rows
    }

    /// Rows per batch, a power of two: row `r` is row `r % batch_rows` of batch
    /// `r / batch_rows`. Every batch but the last is full.
    #[must_use]
    pub fn batch_rows(&self) -> u64 {
        self.shape.batch_rows
    }

    /// The batches, in row order.
    #[must_use]
    pub fn batches(&self) -> &[BatchLayout] {
        &self.batches
    }

    /// The strings of a dictionary-encoded column.
    #[must_use]
    pub fn dictionary(&self) -> Option<&DictionaryLayout> {
        self.dictionary.as_ref()
    }

    /// The length of the column section.
    #[must_use]
    pub fn column_len(&self) -> u64 {
        self.column_len
    }
}

/// Checks that a column of this shape can be written: its limits, and a layout within
/// [`MAX_LAYOUT_BYTES`].
pub(super) fn check(shape: &Shape) -> Result<(), ColumnError> {
    shape.check().map_err(ColumnError::Input)?;
    if layout_len(shape).is_none_or(|len| len > MAX_LAYOUT_BYTES) {
        return Err(ColumnError::TooLarge("the layout would exceed 16 MiB"));
    }
    Ok(())
}

/// The length of a layout section for this shape; `None` past `u64`.
fn layout_len(shape: &Shape) -> Option<u64> {
    let dictionary = if shape.dictionary.is_some() {
        DICTIONARY
    } else {
        0
    };
    let fixed = HEADER + shape.name.len().next_multiple_of(8) + dictionary;
    shape
        .batches()
        .checked_mul(BATCH as u64)?
        .checked_add(fixed as u64)
}

fn encode_type(ty: ColumnType) -> (u8, u32, u32) {
    let int = |bits, signed| (1, bits, u32::from(signed));
    // The scale is an `i32` stored in two's complement.
    let decimal = |code, precision: u8, scale: i8| {
        (code, u32::from(precision), i32::from(scale).cast_unsigned())
    };
    match ty {
        ColumnType::Int8 => int(8, true),
        ColumnType::Int16 => int(16, true),
        ColumnType::Int32 => int(32, true),
        ColumnType::Int64 => int(64, true),
        ColumnType::UInt8 => int(8, false),
        ColumnType::UInt16 => int(16, false),
        ColumnType::UInt32 => int(32, false),
        ColumnType::UInt64 => int(64, false),
        ColumnType::Float32 => (2, 32, 0),
        ColumnType::Float64 => (2, 64, 0),
        ColumnType::Bool => (3, 0, 0),
        ColumnType::Timestamp(unit) => (
            4,
            match unit {
                TimeUnit::Second => 0,
                TimeUnit::Millisecond => 1,
                TimeUnit::Microsecond => 2,
                TimeUnit::Nanosecond => 3,
            },
            0,
        ),
        ColumnType::FixedSizeBinary(width) => (5, width, 0),
        ColumnType::Decimal128 { precision, scale } => decimal(6, precision, scale),
        ColumnType::Decimal256 { precision, scale } => decimal(7, precision, scale),
        ColumnType::Dictionary(width) => (
            DICTIONARY_CODE,
            match width {
                CodeWidth::U8 => 8,
                CodeWidth::U16 => 16,
                CodeWidth::U32 => 32,
            },
            0,
        ),
    }
}

/// The type with these codes, if this version defines it; its parameters are checked
/// by [`Shape::check`].
fn decode_type(code: u8, p1: u32, p2: u32) -> Option<ColumnType> {
    let decimal = |precision| {
        Some((
            u8::try_from(precision).ok()?,
            i8::try_from(p2.cast_signed()).ok()?,
        ))
    };
    let ty = match (code, p1, p2) {
        (1, 8, 1) => ColumnType::Int8,
        (1, 16, 1) => ColumnType::Int16,
        (1, 32, 1) => ColumnType::Int32,
        (1, 64, 1) => ColumnType::Int64,
        (1, 8, 0) => ColumnType::UInt8,
        (1, 16, 0) => ColumnType::UInt16,
        (1, 32, 0) => ColumnType::UInt32,
        (1, 64, 0) => ColumnType::UInt64,
        (2, 32, 0) => ColumnType::Float32,
        (2, 64, 0) => ColumnType::Float64,
        (3, 0, 0) => ColumnType::Bool,
        (4, 0, 0) => ColumnType::Timestamp(TimeUnit::Second),
        (4, 1, 0) => ColumnType::Timestamp(TimeUnit::Millisecond),
        (4, 2, 0) => ColumnType::Timestamp(TimeUnit::Microsecond),
        (4, 3, 0) => ColumnType::Timestamp(TimeUnit::Nanosecond),
        (5, width, 0) => ColumnType::FixedSizeBinary(width),
        (6, p, _) => {
            let (precision, scale) = decimal(p)?;
            ColumnType::Decimal128 { precision, scale }
        }
        (7, p, _) => {
            let (precision, scale) = decimal(p)?;
            ColumnType::Decimal256 { precision, scale }
        }
        (DICTIONARY_CODE, 8, 0) => ColumnType::Dictionary(CodeWidth::U8),
        (DICTIONARY_CODE, 16, 0) => ColumnType::Dictionary(CodeWidth::U16),
        (DICTIONARY_CODE, 32, 0) => ColumnType::Dictionary(CodeWidth::U32),
        _ => return None,
    };
    Some(ty)
}
