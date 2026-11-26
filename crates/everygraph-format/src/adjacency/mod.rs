// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The compressed adjacency codec (DESIGN D4, `docs/spec/eg-adjacency.md`).
//!
//! An adjacency of `n` sorted lists is stored as two parts:
//!
//! - **data**: one record per non-empty list, in node order — Stream VByte of the
//!   zigzag-coded first neighbour (relative to the node) and of the gaps minus one;
//! - **index**: a header, node-aligned blocks of about 64 Ki entries, and two
//!   Elias–Fano sequences of `n + 1` values each: the byte offset of every record and the
//!   entry offset (cumulative degree) of every list.
//!
//! Degrees and entry offsets come from the index alone. A list is found through its
//! byte offset, and decoded by the scalar decoder or by a SIMD one ([`Decoder`]); all
//! of them return the same bytes.
//!
//! A list holds each neighbour once (a pair); when some pair has more than one event,
//! a third part, the **events** ([`AdjacencyEvents`], announced by index flag bit 1),
//! gives each entry's event offsets (ADR 0028).
//!
//! [`encode`] encodes a CSR held in memory. A build that does not hold the graph writes
//! the same bytes list by list: [`append_record`] for the data, [`IndexBuilder`] for the
//! index and [`encode_events`] for the events.
//!
//! ```
//! use everygraph_core::NodeId;
//! use everygraph_format::adjacency::{AdjacencyIndex, Adjacency, Batch, encode};
//!
//! // Node 0 lists 1 and 2; node 1 lists 0; node 2 lists 0.
//! let encoded = encode(&[0, 2, 3, 4], &[1, 2, 0, 0]).unwrap();
//! let index = AdjacencyIndex::parse(&encoded.index).unwrap();
//! let adjacency = Adjacency::new(&index, &encoded.data).unwrap();
//! assert_eq!(index.degree(NodeId::new(0)), 2);
//!
//! let mut buf = Vec::new();
//! assert_eq!(adjacency.neighbors(NodeId::new(0), &mut buf).unwrap(), &[1, 2]);
//!
//! let mut batch = Batch::default();
//! assert_eq!(adjacency.decode_nodes(0..3, 8192, &mut batch).unwrap(), 3);
//! assert_eq!(batch.list(2), &[0]);
//! ```

mod decode;
mod ef;
mod encode;
mod events;
mod index;
mod simd;
mod stream;
mod svb;

use std::error::Error;
use std::fmt;

pub use decode::{Adjacency, Batch};
pub use encode::{Encoded, encode};
pub use events::AdjacencyEvents;
pub use index::AdjacencyIndex;
pub use simd::Decoder;
pub use stream::{
    IndexBuilder, append_record, encode_events, events_section_bytes, index_bytes_bound,
    record_bound,
};

/// Why index bytes were refused by [`AdjacencyIndex::parse`] or data bytes by
/// [`Adjacency::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FormatError {
    /// Shorter than the header.
    Truncated,
    /// Not an adjacency index.
    BadMagic,
    /// A format version this reader does not know.
    UnsupportedVersion(u32),
    /// Feature flags this reader does not know.
    UnsupportedFlags(u32),
    /// A block coded with a codec this reader does not know (version 1 reserves the
    /// field for an archival codec).
    UnsupportedBlockCodec {
        /// The block.
        block: u64,
        /// Its codec byte.
        codec: u8,
    },
    /// The bytes do not have the length the header implies.
    Length {
        /// Length the header implies.
        expected: u64,
        /// Length given.
        actual: u64,
    },
    /// Structurally invalid.
    Invalid(&'static str),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "adjacency index shorter than its header"),
            Self::BadMagic => write!(f, "not an adjacency index (bad magic)"),
            Self::UnsupportedVersion(v) => {
                write!(f, "adjacency format version {v} is not supported")
            }
            Self::UnsupportedFlags(x) => {
                write!(f, "adjacency format flags {x:#x} are not supported")
            }
            Self::UnsupportedBlockCodec { block, codec } => {
                write!(
                    f,
                    "block {block} uses codec {codec}, which is not supported"
                )
            }
            Self::Length { expected, actual } => {
                write!(
                    f,
                    "adjacency part is {actual} bytes, its header implies {expected}"
                )
            }
            Self::Invalid(what) => write!(f, "invalid adjacency index: {what}"),
        }
    }
}

impl Error for FormatError {}

/// Why [`encode`] refused its input.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodeError {
    /// The offsets are not those of a CSR over the targets given.
    Offsets(&'static str),
    /// The list of this node is not strictly increasing.
    NotIncreasing {
        /// The node.
        node: u32,
    },
    /// A buffer of the encoder could not be allocated.
    OutOfMemory {
        /// Its size.
        bytes: u64,
    },
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Offsets(what) => write!(f, "invalid CSR offsets: {what}"),
            Self::NotIncreasing { node } => {
                write!(f, "the list of node {node} is not strictly increasing")
            }
            Self::OutOfMemory { bytes } => write!(f, "could not allocate {bytes} bytes to encode"),
        }
    }
}

impl Error for EncodeError {}

/// Records that do not decode to what the index says: the data part is corrupt, or
/// does not belong to this index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeError {
    /// First node of the range in which the problem was found.
    pub first: u32,
    /// One past its last node.
    pub end: u32,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "corrupt adjacency records in nodes {}..{}",
            self.first, self.end
        )
    }
}

impl Error for DecodeError {}

/// Proptest settings of the unit tests: few cases under Miri, and no failure file
/// (Miri's isolation hides the file system).
#[cfg(test)]
fn test_config(miri_cases: u32, cases: u32) -> proptest::test_runner::Config {
    let default = proptest::test_runner::Config::default();
    proptest::test_runner::Config {
        cases: if cfg!(miri) { miri_cases } else { cases },
        failure_persistence: if cfg!(miri) {
            None
        } else {
            default.failure_persistence.clone()
        },
        ..default
    }
}
