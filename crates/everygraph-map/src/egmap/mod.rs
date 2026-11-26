// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `.egmap` map file (`docs/spec/egmap.md`): a PMTiles v3 archive of data tiles,
//! with a table of chunked arrays after the root directory.
//!
//! - [`Tile`] is the content of one tile, with [`Tile::encode`] and [`Tile::decode`]
//!   for its payload (§ 4).
//! - [`MapWriter`] writes a file from tiles in tile-id order, then arrays (§ 5).
//! - [`MapReader`] opens a file through a [`ByteSource`](everygraph_core::ByteSource),
//!   finds tiles and reads chunks; [`MapReader::verify`] reads all of it.
//! - [`Census`] counts the zoom-consistency faults of a set of tiles (§ 6).
//!
//! Every size a file declares is checked against the bytes that hold it before
//! anything is allocated for it, and nothing decodes past the caps of § 1.

mod census;
mod gzip;
mod pmtiles;
mod read;
mod sections;
mod tile;
mod write;

use std::error::Error;
use std::fmt;

use everygraph_core::{MemoryError, ReadError, WriteError};

pub use census::{Census, CensusReport, ZoomRule, min_zoom};
pub use pmtiles::{tile_id, tile_of_id};
pub use read::MapReader;
pub use sections::{Element, Section, Values};
pub use tile::{Anchor, Label, Node, Omitted, Region, RegionKind, Tile};
pub use write::{MapInfo, MapWriter, Metadata};

/// Width of a tile in pixels: the unit of every pixel threshold (§ 3.1).
pub const TILE_PX: u32 = 256;
/// The deepest zoom a file may have. A tile at zoom 20 spans 2^-20 of the map, and the
/// pixels of a tile at zoom 16 are already the spacing of `f32` values near 1.
pub const MAX_ZOOM: u8 = 20;
/// The most bytes a tile payload, a directory, the metadata or a chunk decodes to.
pub const MAX_DECODED_BYTES: usize = 1 << 20;
/// Bytes at the start of the file that hold the header, the root directory and the
/// section table: what one request of a PMTiles client fetches.
pub const HEAD_BYTES: u64 = 16_384;
const HEAD_LEN: usize = 16_384;

/// Why a map file could not be read or written.
#[derive(Debug)]
#[non_exhaustive]
pub enum EgmapError {
    /// The source failed.
    Read(ReadError),
    /// The sink failed.
    Write(WriteError),
    /// The memory budget refused a buffer.
    Memory(MemoryError),
    /// Not a PMTiles v3 archive, or one that is not a map.
    Foreign(&'static str),
    /// A later version, or a required tile block this reader does not know.
    Unsupported(String),
    /// The bytes break a rule of the specification.
    Malformed(String),
    /// What the writer was given breaks a rule of the specification.
    Invalid(String),
}

impl fmt::Display for EgmapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(e) => write!(f, "reading the map: {e}"),
            Self::Write(e) => write!(f, "writing the map: {e}"),
            Self::Memory(e) => e.fmt(f),
            Self::Foreign(what) => write!(f, "not a map file: {what}"),
            Self::Unsupported(what) => write!(f, "a later map format: {what}"),
            Self::Malformed(what) => write!(f, "damaged map file: {what}"),
            Self::Invalid(what) => write!(f, "cannot write this map: {what}"),
        }
    }
}

impl Error for EgmapError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read(e) => Some(e),
            Self::Write(e) => Some(e),
            Self::Memory(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ReadError> for EgmapError {
    fn from(e: ReadError) -> Self {
        Self::Read(e)
    }
}

impl From<WriteError> for EgmapError {
    fn from(e: WriteError) -> Self {
        Self::Write(e)
    }
}

impl From<MemoryError> for EgmapError {
    fn from(e: MemoryError) -> Self {
        Self::Memory(e)
    }
}

fn malformed(what: impl Into<String>) -> EgmapError {
    EgmapError::Malformed(what.into())
}

fn invalid(what: impl Into<String>) -> EgmapError {
    EgmapError::Invalid(what.into())
}
