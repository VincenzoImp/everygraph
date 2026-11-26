// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Property columns (DESIGN D9, ADR 0009 and 0028, `docs/spec/eg-columns.md`).
//!
//! A column is stored as up to three container sections:
//!
//! - the **column** ([`Kind::COLUMN`]): a complete Arrow IPC file, which `pyarrow`,
//!   Polars or any Arrow reader opens without Everygraph. Its framing is written here,
//!   not by an Arrow library, so that its bytes are a function of the values alone; every
//!   buffer starts at a multiple of 4096 in the file;
//! - its **layout** ([`Kind::COLUMN_LAYOUT`]): the type, the batches and the offset of
//!   every buffer, in a fixed binary record, so that a kernel finds the values without
//!   parsing FlatBuffers;
//! - its **statistics** ([`Kind::COLUMN_STATS`]), for numeric columns: count, nulls,
//!   NaNs, minimum, maximum and the exact sum rounded once (`graph-semantics.md` § 2.7).
//!
//! [`ColumnWriter`] writes all three into a container [`Writer`]; [`ColumnLayout`],
//! [`Column`] and [`ColumnStats`] read them. The reader checks that the column's framing
//! is exactly the one its layout implies, so what an Arrow reader sees and what
//! Everygraph reads are the same values.
//!
//! ```
//! use everygraph_core::MemorySource;
//! use everygraph_format::columns::{
//!     Column, ColumnLayout, ColumnSpec, ColumnStats, ColumnType, SectionNames, write_column,
//! };
//! use everygraph_format::container::{Container, Writer};
//! use everygraph_core::ByteSource;
//!
//! let mut writer = Writer::create(MemorySource::new(Vec::new())).unwrap();
//! let spec = ColumnSpec::new("weight", ColumnType::Float64, 3);
//! write_column(&mut writer, "adjacency.out.pair", &spec, &[0.5, 2.0, 1.5], None).unwrap();
//! writer.commit().unwrap();
//!
//! let file = Container::open(writer.into_sink()).unwrap();
//! let names = SectionNames::new("adjacency.out.pair", "weight");
//! let mut bytes = vec![0; file.get(&names.layout).unwrap().len as usize];
//! file.section(&names.layout).unwrap().read_at(0, &mut bytes).unwrap();
//! let layout = ColumnLayout::parse(&bytes).unwrap();
//! let column = Column::open(&layout, file.section(&names.column).unwrap()).unwrap();
//! let values = &layout.batches()[0].values;
//! assert_eq!(values.start % 4096, 0);
//! let mut raw = [0; 8];
//! column.source().read_at(values.start + 8, &mut raw).unwrap();
//! assert_eq!(f64::from_le_bytes(raw), 2.0);
//! ```

mod arrow;
mod flatbuf;
mod layout;
mod read;
mod stats;
mod write;

use std::error::Error;
use std::fmt;

use everygraph_core::ReadError;

use crate::container::ContainerError;
#[cfg(doc)]
use crate::container::{Kind, Writer};

pub use layout::{BatchLayout, ColumnLayout, DictionaryLayout};
pub use read::Column;
pub use stats::ColumnStats;
pub use write::{Bits, ColumnSpec, ColumnWriter, Dictionary, Values, write_column};

/// The largest layout section a reader accepts, and a writer writes: 16 MiB, about
/// 400,000 batches (§ 3.1, § 4).
pub const MAX_LAYOUT_BYTES: u64 = 16 << 20;
/// The longest column name, in bytes of UTF-8.
pub const MAX_NAME_BYTES: usize = 512;
/// The largest batch row count: 2^30 rows (§ 3.1).
pub const MAX_BATCH_ROWS: u64 = 1 << 30;
/// A batch body, and the dictionary's, is below this: 2 GiB (§ 3.1).
pub const MAX_BODY_BYTES: u64 = 1 << 31;

/// The unit of a timestamp column. Every timestamp column is in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeUnit {
    /// Seconds.
    Second,
    /// Milliseconds.
    Millisecond,
    /// Microseconds.
    Microsecond,
    /// Nanoseconds.
    Nanosecond,
}

impl TimeUnit {
    /// The unit's code: 0 s, 1 ms, 2 us, 3 ns (Arrow's `TimeUnit`; spec § 2).
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Self::Second => 0,
            Self::Millisecond => 1,
            Self::Microsecond => 2,
            Self::Nanosecond => 3,
        }
    }
}

/// The width of the codes of a dictionary-encoded column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CodeWidth {
    /// `uint8` codes: up to 256 values.
    U8,
    /// `uint16` codes: up to 65,536 values.
    U16,
    /// `uint32` codes: up to 2^31 − 1 values (an Arrow array's limit).
    U32,
}

impl CodeWidth {
    /// Bits per code: 8, 16 or 32.
    #[must_use]
    pub fn bits(self) -> u8 {
        match self {
            Self::U8 => 8,
            Self::U16 => 16,
            Self::U32 => 32,
        }
    }

    /// The most strings the codes can name.
    #[must_use]
    pub fn most_values(self) -> u64 {
        match self {
            Self::U8 => 1 << 8,
            Self::U16 => 1 << 16,
            Self::U32 => (1 << 31) - 1,
        }
    }
}

/// The type of a column (`docs/spec/graph-semantics.md` § 2.6; `eg-columns.md` § 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColumnType {
    /// `int8`.
    Int8,
    /// `int16`.
    Int16,
    /// `int32`.
    Int32,
    /// `int64`.
    Int64,
    /// `uint8`.
    UInt8,
    /// `uint16`.
    UInt16,
    /// `uint32`.
    UInt32,
    /// `uint64`.
    UInt64,
    /// `float32`.
    Float32,
    /// `float64`.
    Float64,
    /// `bool`, one bit per row.
    Bool,
    /// `timestamp[unit, tz=UTC]`, an `i64` per row.
    Timestamp(TimeUnit),
    /// `fixed_size_binary[width]`, 1 to 2^20 bytes per row.
    FixedSizeBinary(u32),
    /// `decimal128(precision, scale)`: precision 1–38, scale −76 to 76.
    Decimal128 {
        /// Decimal digits.
        precision: u8,
        /// The value is the stored integer times 10^−scale.
        scale: i8,
    },
    /// `decimal256(precision, scale)`: precision 1–76, scale −76 to 76.
    Decimal256 {
        /// Decimal digits.
        precision: u8,
        /// The value is the stored integer times 10^−scale.
        scale: i8,
    },
    /// Dictionary-encoded UTF-8 text: a code per row, the strings once.
    Dictionary(CodeWidth),
}

impl ColumnType {
    /// The type of an edge id over `events` stored events: `uint32` below 2^32 events,
    /// `uint64` from there (ADR 0028).
    #[must_use]
    pub fn edge_id(events: u64) -> Self {
        if events < 1 << 32 {
            Self::UInt32
        } else {
            Self::UInt64
        }
    }

    /// Bytes per row; 0 for [`Bool`](Self::Bool), which takes one bit.
    #[must_use]
    pub fn width(self) -> u64 {
        match self {
            Self::Bool => 0,
            Self::Int8 | Self::UInt8 | Self::Dictionary(CodeWidth::U8) => 1,
            Self::Int16 | Self::UInt16 | Self::Dictionary(CodeWidth::U16) => 2,
            Self::Int32 | Self::UInt32 | Self::Float32 | Self::Dictionary(CodeWidth::U32) => 4,
            Self::Int64 | Self::UInt64 | Self::Float64 | Self::Timestamp(_) => 8,
            Self::Decimal128 { .. } => 16,
            Self::Decimal256 { .. } => 32,
            Self::FixedSizeBinary(w) => u64::from(w),
        }
    }

    /// Whether the column has statistics (§ 5): every integer, float, timestamp and
    /// decimal type.
    #[must_use]
    pub fn is_numeric(self) -> bool {
        !matches!(
            self,
            Self::Bool | Self::FixedSizeBinary(_) | Self::Dictionary(_)
        )
    }

    /// The batch row count a writer uses unless told otherwise: 2^26 rows, fewer for
    /// values wider than 16 bytes, so that a batch holds at most 1 GiB of values.
    #[must_use]
    pub fn default_batch_rows(self) -> u64 {
        let width = self.width().max(1);
        let rows = (1u64 << 30) / width;
        (1 << rows.ilog2()).min(1 << 26)
    }

    /// The decimal scale; 0 for every other type.
    pub(super) fn scale(self) -> i8 {
        match self {
            Self::Decimal128 { scale, .. } | Self::Decimal256 { scale, .. } => scale,
            _ => 0,
        }
    }

    /// Checks the parameters of the type.
    fn check(self) -> Result<(), &'static str> {
        let ok = match self {
            Self::FixedSizeBinary(w) => (1..=1 << 20).contains(&w),
            Self::Decimal128 { precision, scale } => {
                (1..=38).contains(&precision) && (-76..=76).contains(&scale)
            }
            Self::Decimal256 { precision, scale } => {
                (1..=76).contains(&precision) && (-76..=76).contains(&scale)
            }
            _ => true,
        };
        if ok {
            Ok(())
        } else {
            Err("a type parameter is out of range")
        }
    }
}

/// The three section names of a column (`docs/spec/eg-columns.md` § 6):
/// `<scope>/arrow/<column>`, `<scope>/layout/<column>`, `<scope>/stats/<column>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionNames {
    /// The Arrow IPC file.
    pub column: String,
    /// Its layout.
    pub layout: String,
    /// Its statistics (written for numeric columns only).
    pub stats: String,
}

impl SectionNames {
    /// The names of column `column` in scope `scope` (`nodes`, `edges`, `<adj>.pair`,
    /// `<adj>.event`).
    #[must_use]
    pub fn new(scope: &str, column: &str) -> Self {
        Self {
            column: format!("{scope}/arrow/{column}"),
            layout: format!("{scope}/layout/{column}"),
            stats: format!("{scope}/stats/{column}"),
        }
    }
}

/// Why a column could not be written or read.
#[derive(Debug)]
#[non_exhaustive]
pub enum ColumnError {
    /// The container refused a section, or its sink failed.
    Container(ContainerError),
    /// The source failed.
    Read(ReadError),
    /// A layout, column or statistics section breaks the specification.
    Invalid(&'static str),
    /// A version or flag this reader does not know.
    Unsupported(&'static str),
    /// The writer was used against its contract (wrong row count, wrong type).
    Input(&'static str),
    /// Past a limit of the format.
    TooLarge(&'static str),
}

impl fmt::Display for ColumnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Container(e) => write!(f, "{e}"),
            Self::Read(e) => write!(f, "cannot read a property column: {e}"),
            Self::Invalid(what) => write!(f, "invalid property column: {what}"),
            Self::Unsupported(what) => write!(f, "unsupported property column: {what}"),
            Self::Input(what) => write!(f, "cannot write the property column: {what}"),
            Self::TooLarge(what) => write!(f, "property column too large: {what}"),
        }
    }
}

impl Error for ColumnError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Container(e) => Some(e),
            Self::Read(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ContainerError> for ColumnError {
    fn from(e: ContainerError) -> Self {
        Self::Container(e)
    }
}

impl From<ReadError> for ColumnError {
    fn from(e: ReadError) -> Self {
        Self::Read(e)
    }
}
