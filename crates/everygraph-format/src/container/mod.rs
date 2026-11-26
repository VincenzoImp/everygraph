// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `.eg` container (DESIGN D7, ADR 0007 and 0027, `docs/spec/eg-container.md`).
//!
//! A graph file is a set of named, content-hashed **sections** found through a **table
//! of contents** (TOC) at the end of what was last committed, which one of two **header
//! blocks** at the start points to. In bytes:
//!
//! ```text
//! 0      4096   header block 0 } the newest valid one points to the TOC
//! 4096   8192   header block 1 }
//! 8192   …      sections, each at a multiple of 4096, each followed by ≥ 16 zero bytes
//!               the table of contents (the same alignment and padding), last
//! ```
//!
//! **Reading** ([`Container::open`]) costs two positional reads, whatever the size of
//! the file: the two header blocks and the TOC. A section is then read through
//! [`Container::section`], a [`ByteSource`](everygraph_core::ByteSource) over its
//! bytes, and checked against its BLAKE3 hash with [`Container::verify`].
//!
//! **Writing** ([`Writer`]) appends sections after the committed end and never touches
//! a byte that a committed TOC refers to. [`Writer::commit`] writes a new TOC, makes
//! everything durable, then writes the header block that is *not* the newest one and
//! makes that durable: until the second write is durable the previous TOC is the file's
//! state, and a torn header block fails its hash and is ignored. [`compact`] copies the
//! live sections into a new container to reclaim what was replaced or abandoned.
//!
//! **Results** (spec § 6) are sections of typed values whose provenance record holds a
//! [`ResultKey`]: the [`GraphIdentity`] of the graph they were computed on, the
//! operation and its version, determinism, and the parameters.
//! [`Writer::add_result`] appends one (replacing one under the same key);
//! [`Container::find_result`] serves it only for that key on that graph.
//!
//! ```
//! use everygraph_core::MemorySource;
//! use everygraph_format::container::{Container, Kind, Provenance, Writer};
//!
//! let mut writer = Writer::create(MemorySource::new(Vec::new())).unwrap();
//! let record = Provenance { engine: "everygraph 0.0.0".into(), ..Provenance::default() };
//! writer.add_provenance("provenance/0", &record).unwrap();
//! writer.add_section("note", Kind::new(0x8000_0000).unwrap(), b"hello", Some("provenance/0")).unwrap();
//! writer.commit().unwrap();
//!
//! let file = Container::open(writer.into_sink()).unwrap();
//! assert_eq!(file.generation(), 1);
//! file.verify("note").unwrap();
//! assert_eq!(file.provenance("note").unwrap().unwrap().engine, "everygraph 0.0.0");
//! ```
//!
//! The crate never opens files: a native file is an `everygraph_io::FileSink` (writing)
//! or any `ByteSource` of `everygraph-io` (reading).

pub(crate) mod layout;
mod provenance;
pub(crate) mod read;
mod result;
mod write;

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use everygraph_core::{ReadError, WriteError};

pub use layout::{MAJOR, MINOR};
pub use provenance::{Input, Provenance};
pub use read::{Container, Section};
pub use result::{GraphIdentity, ResultKey, StoredResult, Value, ValueType};
pub use write::{SectionWriter, Writer, compact};

/// Sections, the TOC and the header blocks start at multiples of this.
pub const BLOCK: u64 = 4096;
/// The two header blocks: sections start at or after this offset.
pub const HEADER_BYTES: u64 = 2 * BLOCK;
/// Zero bytes that follow every section at least, so that SIMD decoders may read past
/// a section's end.
pub const MIN_PADDING: u64 = 16;
/// The largest table of contents a reader accepts (and a writer writes): 16 MiB.
pub const MAX_TOC_BYTES: u64 = 16 << 20;
/// The longest section name, in bytes of UTF-8.
pub const MAX_NAME_BYTES: usize = 1024;
/// The largest provenance record a reader accepts: 16 MiB.
pub const MAX_PROVENANCE_BYTES: u64 = 16 << 20;
/// Required feature bit 0, `segments` (spec § 10, § 13): set exactly when the graph of
/// the file is the union of more than one adjacency run. Reserved: this version does
/// not know it, so it refuses a file that sets it ([`ContainerError::UnsupportedFeatures`])
/// rather than read only the base run, an older graph.
pub const REQUIRED_SEGMENTS: u64 = 1 << 0;
/// Section names that begin with this are reserved for segments (spec § 4, § 13):
/// `seg/` + 8 lowercase hexadecimal digits + `/` + the unsegmented name. This version
/// does not write them.
pub const SEGMENT_PREFIX: &str = "seg/";

/// What a section holds (spec § 5). A reader skips sections whose kind it does not
/// know; that is how later issues add artifacts without a new version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Kind(u32);

impl Kind {
    /// A provenance record ([`Provenance`]), written by [`Writer::add_provenance`].
    pub const PROVENANCE: Self = Self(1);
    /// The index of a compressed adjacency (`docs/spec/eg-adjacency.md` § 3).
    pub const ADJACENCY_INDEX: Self = Self(2);
    /// The data of a compressed adjacency (`docs/spec/eg-adjacency.md` § 1).
    pub const ADJACENCY_DATA: Self = Self(3);
    /// One section of an identifier dictionary (`ids.*`, `everygraph_ingest::ids`).
    pub const ID_DICTIONARY: Self = Self(4);
    /// The event offsets of an adjacency (`<name>.events`): where each pair's events
    /// start, for graphs that keep repeated pairs as events. Specified with the
    /// adjacency format.
    pub const ADJACENCY_EVENTS: Self = Self(5);
    /// The segment manifest of a segmented file (`segments`, spec § 13). Reserved:
    /// version 1.0 neither writes nor reads segments.
    pub const SEGMENTS: Self = Self(6);
    /// A result kept in the graph file: typed values, with a provenance record that
    /// holds its key (spec § 6, "Results"), written by [`Writer::add_result`].
    pub const RESULT: Self = Self(10);
    /// The first of the kinds reserved for private and experimental use, which no
    /// specification will ever assign.
    pub const PRIVATE: Self = Self(0x8000_0000);

    /// The kind with this code; `None` for 0, which no section may have.
    #[must_use]
    pub const fn new(code: u32) -> Option<Self> {
        if code == 0 { None } else { Some(Self(code)) }
    }

    /// The code written in the TOC.
    #[must_use]
    pub const fn code(self) -> u32 {
        self.0
    }

    /// Whether this version of the specification defines the kind.
    #[must_use]
    pub const fn is_known(self) -> bool {
        matches!(self.0, 1..=6 | 10)
    }

    /// Whether sections of this kind define the graph, and so enter its identity
    /// ([`GraphIdentity`], spec § 6).
    #[must_use]
    pub const fn defines_graph(self) -> bool {
        matches!(self.0, 2..=5)
    }
}

/// How a section's stored bytes are encoded (spec § 5). Version 1.0 defines only
/// [`Codec::NONE`]; a section with another codec can be listed, copied and verified,
/// but not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Codec(u16);

impl Codec {
    /// The bytes are the content.
    pub const NONE: Self = Self(0);

    /// The codec with this code.
    #[must_use]
    pub const fn new(code: u16) -> Self {
        Self(code)
    }

    /// The code written in the TOC.
    #[must_use]
    pub const fn code(self) -> u16 {
        self.0
    }

    /// Whether this version of the specification defines the codec.
    #[must_use]
    pub const fn is_known(self) -> bool {
        self.0 == 0
    }
}

/// One entry of the table of contents.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    /// Unique within the TOC; 1 to [`MAX_NAME_BYTES`] bytes of UTF-8. Shared, not
    /// copied, by the entries that refer to it as their provenance.
    pub name: Arc<str>,
    /// What the section holds.
    pub kind: Kind,
    /// How its bytes are encoded.
    pub codec: Codec,
    /// Where it starts: a multiple of [`BLOCK`], at least [`HEADER_BYTES`].
    pub offset: u64,
    /// Its length in bytes, padding not included.
    pub len: u64,
    /// BLAKE3-256 of its `len` stored bytes.
    pub hash: [u8; 32],
    /// The name of the provenance record that says how it was made, if any (never for
    /// a provenance record itself).
    pub provenance: Option<Arc<str>>,
}

/// Why a container could not be read or written.
#[derive(Debug)]
#[non_exhaustive]
pub enum ContainerError {
    /// The source failed.
    Read(ReadError),
    /// The sink failed. A [`Writer`] that saw this refuses to go on
    /// ([`ContainerError::Poisoned`]); the file still reads as its last commit.
    Write(WriteError),
    /// Neither header block has the magic: not an `.eg` file, one shorter than its
    /// header, or a new file whose first commit stopped before its header block.
    NotAContainer,
    /// The magic is there, but no header block is valid: a file whose first commit
    /// stopped while its header block was being written.
    NotCommitted,
    /// Written by a big-endian writer, which this version does not define.
    UnsupportedByteOrder,
    /// A major version this reader does not know.
    UnsupportedVersion {
        /// Major version of the file.
        major: u16,
        /// Minor version of the file.
        minor: u16,
    },
    /// Required feature bits this reader does not know.
    UnsupportedFeatures {
        /// The unknown bits.
        required: u64,
    },
    /// A [`Writer`] does not update a file of a newer minor version than its own: it
    /// could not keep what that minor added.
    NewerMinor {
        /// Minor version of the file.
        minor: u16,
    },
    /// The header or the table of contents breaks a rule of the specification.
    Corrupt(&'static str),
    /// A section's bytes do not match its hash, or its padding is not zero.
    Damaged {
        /// The section.
        name: String,
    },
    /// A section's codec is not one this reader can decode.
    UnsupportedCodec {
        /// The section.
        name: String,
        /// Its codec.
        codec: Codec,
    },
    /// No section has this name.
    NoSuchSection(String),
    /// A section with this name is already in the table being written.
    NameTaken(String),
    /// A name that is empty or longer than [`MAX_NAME_BYTES`].
    InvalidName,
    /// A provenance reference to a section that is not a provenance record, or a
    /// provenance record given to [`Writer::begin_section`] instead of
    /// [`Writer::add_provenance`].
    NotProvenance(String),
    /// A provenance record that another section still refers to cannot be removed.
    StillReferenced {
        /// The record.
        name: String,
        /// A section that refers to it.
        by: String,
    },
    /// A provenance record that cannot be encoded or decoded.
    InvalidProvenance(&'static str),
    /// A result section that breaks its layout, or a result that cannot be stored or
    /// read as asked (spec § 6, "Results").
    InvalidResult(&'static str),
    /// The table of contents or a record would exceed its limit.
    TooLarge(&'static str),
    /// An earlier write failed: this writer does nothing more.
    Poisoned,
    /// [`Writer::update`] refuses a file that was opened at an older commit than the
    /// newest one it may hold ([`Container::recovered_from`],
    /// [`Container::unreadable_block`]): its first commit would write over the newer
    /// commit's bytes. [`Writer::update_recovered`] does it on purpose.
    Recovered,
    /// The generation counter would pass 2^64 − 1.
    GenerationExhausted,
    /// A name or kind reserved for segments ([`SEGMENT_PREFIX`], [`Kind::SEGMENTS`]),
    /// which this version does not write.
    Reserved(String),
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(e) => write!(f, "cannot read the graph file: {e}"),
            Self::Write(e) => write!(f, "cannot write the graph file: {e}"),
            Self::NotAContainer => write!(f, "not an Everygraph graph file (no .eg header)"),
            Self::NotCommitted => write!(
                f,
                "the graph file was never completed: it has no valid header block"
            ),
            Self::UnsupportedByteOrder => {
                write!(f, "the graph file is big-endian, which is not supported")
            }
            Self::UnsupportedVersion { major, minor } => write!(
                f,
                "the graph file has format version {major}.{minor}; this Everygraph reads {MAJOR}.x"
            ),
            Self::UnsupportedFeatures { required } if required & REQUIRED_SEGMENTS != 0 => {
                write!(
                    f,
                    "the graph file was built by appends (segments), which this Everygraph \
                     cannot read (required feature bits {required:#x})"
                )
            }
            Self::UnsupportedFeatures { required } => write!(
                f,
                "the graph file needs features this Everygraph does not have (bits {required:#x})"
            ),
            Self::NewerMinor { minor } => write!(
                f,
                "the graph file has format version {MAJOR}.{minor}, newer than {MAJOR}.{MINOR}: \
                 this Everygraph can read it but not add to it"
            ),
            Self::Corrupt(what) => write!(f, "the graph file is damaged: {what}"),
            Self::Damaged { name } => write!(
                f,
                "section {name:?} of the graph file is damaged: its bytes do not match its hash"
            ),
            Self::UnsupportedCodec { name, codec } => write!(
                f,
                "section {name:?} is encoded with codec {}, which this Everygraph cannot read",
                codec.code()
            ),
            Self::NoSuchSection(name) => write!(f, "the graph file has no section {name:?}"),
            Self::NameTaken(name) => write!(f, "a section {name:?} is already being written"),
            Self::InvalidName => write!(
                f,
                "a section name must be 1 to {MAX_NAME_BYTES} bytes of UTF-8"
            ),
            Self::NotProvenance(name) => {
                write!(f, "section {name:?} is not a provenance record")
            }
            Self::StillReferenced { name, by } => write!(
                f,
                "provenance record {name:?} cannot be removed: section {by:?} refers to it"
            ),
            Self::InvalidProvenance(what) => write!(f, "invalid provenance record: {what}"),
            Self::InvalidResult(what) => write!(f, "invalid result: {what}"),
            Self::TooLarge(what) => write!(f, "too large: {what}"),
            Self::Poisoned => write!(
                f,
                "an earlier write to the graph file failed; it still reads as its last commit"
            ),
            Self::Recovered => write!(
                f,
                "the graph file was opened at an older commit because a newer one could not \
                 be read; adding to it would overwrite that newer commit"
            ),
            Self::Reserved(name) => write!(
                f,
                "section {name:?}: names beginning with \"seg/\" and the segment manifest are \
                 reserved for segments, which this Everygraph does not write"
            ),
            Self::GenerationExhausted => {
                write!(
                    f,
                    "the graph file has reached the last commit number (2^64 - 1)"
                )
            }
        }
    }
}

impl Error for ContainerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read(e) => Some(e),
            Self::Write(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ReadError> for ContainerError {
    fn from(e: ReadError) -> Self {
        Self::Read(e)
    }
}

impl From<WriteError> for ContainerError {
    fn from(e: WriteError) -> Self {
        Self::Write(e)
    }
}

/// Entry points for the fuzz targets (`crates/everygraph-format/fuzz`), built only by
/// `cargo fuzz`, which sets `cfg(fuzzing)`: the decoders of the table of contents and of
/// provenance records, without the hashes that keep a fuzzer's bytes from reaching them.
#[cfg(fuzzing)]
#[doc(hidden)]
pub mod fuzzing {
    use super::{ContainerError, Entry, Provenance};

    /// Decodes a table of contents that starts at `toc_offset`.
    ///
    /// # Errors
    ///
    /// As `Container::open` for a table that breaks the specification.
    pub fn decode_toc(bytes: &[u8], toc_offset: u64) -> Result<Vec<Entry>, ContainerError> {
        super::layout::decode_toc(bytes, toc_offset)
    }

    /// Encodes entries that [`decode_toc`] returned.
    ///
    /// # Errors
    ///
    /// A table that would exceed 16 MiB.
    pub fn encode_toc(entries: &[Entry]) -> Result<Vec<u8>, ContainerError> {
        super::layout::encode_toc(entries.iter())
    }

    /// Decodes a provenance record.
    ///
    /// # Errors
    ///
    /// As `Container::read_provenance` for a record that cannot be decoded.
    pub fn decode_provenance(bytes: &[u8]) -> Result<Provenance, ContainerError> {
        Provenance::decode(bytes)
    }

    /// Encodes a provenance record.
    ///
    /// # Errors
    ///
    /// As `Writer::add_provenance` for a record that cannot be encoded.
    pub fn encode_provenance(record: &Provenance) -> Result<Vec<u8>, ContainerError> {
        record.encode()
    }
}

/// Checks a section name.
fn check_name(name: &str) -> Result<(), ContainerError> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        Err(ContainerError::InvalidName)
    } else {
        Ok(())
    }
}
