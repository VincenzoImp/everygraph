// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The identifier dictionary: the keys of the input (addresses, user names, URLs,
//! integers) and the dense node ids `0..n` the engine works with, in both directions
//! (DESIGN D8, ADR 0008; issue #34).
//!
//! # Use
//!
//! [`build_dictionary`] reads a [`KeySource`] (any chunked, re-readable input) and
//! returns a [`Built`]: the [`Dictionary`] and the keys that were invalid rows.
//! [`Dictionary::lookup`] maps a key to its id, [`Dictionary::key`] an id back to its
//! key, in the type of the input column ([`ColumnType`], `docs/api-sketch.md` § 2.2). A
//! pipeline that reads its input itself drives a [`Collector`] directly: a [`Worker`]
//! per chunk, then [`Collector::finish`] and [`Distinct::build`].
//!
//! # Keys
//!
//! The kind of the keys is chosen from the first [`SAMPLE_SIZE`] keys of the input
//! ([`KeyKind::detect`]: a kind that fits 99 % of them) or given
//! ([`BuildOptions::kind`]; `ids="string"` is [`KeyKind::Bytes`]). A key that does not
//! fit is an invalid row — left out and reported with its position
//! ([`InvalidKeys`]) — never a reason to read the column another way
//! (`design/DECISIONS.md`, "Identifier columns"). Integers are stored as 8 bytes,
//! hexadecimal text, UUIDs and base58 as their decoded bytes, fixed-size binary values
//! as they are, all at a fixed width with no offsets; anything else as bytes (at most
//! [`MAX_KEY_BYTES`]) with offsets. Hexadecimal keys are matched in any case and given
//! back in lowercase, with `0x` when the input had a prefix (`0x` or `0X`); UUIDs in
//! lowercase.
//!
//! # Graph order
//!
//! The ids are graph order (`docs/api-sketch.md` § 2.2, § 2.12): node `i` is the `i`-th
//! row of `g.nodes()`. [`IdOrder`] says how it is assigned — by first appearance in the
//! input, or by sorted key; [`IdOrder::Natural`] (the default) is sorted for integer
//! keys and first appearance otherwise. It is a function of the input and the options
//! only: the same at any thread count and memory budget, because every step that
//! depends on threads or budget (how keys are spread over workers, when tables spill,
//! how work is cut into tasks) is followed by one that orders by key, hash or position.
//! [`Dictionary::renumber`] changes the internal ids later and records each node's
//! graph position ([`Dictionary::graph_position`]), which never changes.
//!
//! An input with 2^32 or more distinct keys is refused ([`DictionaryError::TooManyNodes`],
//! ADR 0003) as soon as the count is exact: after each unit of phase B, or when the
//! bitmap of dense integers is counted. Ids beyond 32 bits are issue #114.
//!
//! # Dense integers
//!
//! Integer keys in sorted order are read into a bitmap over `0..2^32` (from the
//! column's zero: 0 for unsigned columns and text, `i64` 0 for signed ones). They are
//! kept as a rank bitmap when the largest key `m` satisfies `m + 1 ≤ 64 · n` — the
//! bitmap then takes at most 8 bytes per key, less than the 12.3 bytes per key of the
//! hashed form of 8-byte keys — and otherwise go through the hash function like any
//! other kind (same ids). A key outside `0..2^32`, or a bitmap the budget cannot hold,
//! sends the build to the hashed form directly.
//!
//! # How a dictionary is built
//!
//! 1. **Distinct keys.** Each worker puts keys into its own hash table (key, smallest
//!    position seen). When the table cannot grow within the memory budget, it is
//!    written to the [`Scratch`] as a *run*, grouped by the top 10 bits of the key hash
//!    (1,024 fine partitions), and emptied. One key in 1,024 (by hash) is sampled.
//! 2. **Phase B, per unit** (a range of fine partitions): the unit's records are read
//!    from every run and deduplicated; the hash function's parts in the unit are
//!    solved; each key goes to the phase-C bucket of its order value (position, or key)
//!    with its slot.
//! 3. **Phase C, per bucket** (a range of the order value cut at quantiles of the
//!    sample): records are sorted and get consecutive ids; the arrays in id order are
//!    written; each key goes to phase D's range of its slot.
//! 4. **Phase D, per slot range:** the arrays in slot order are written.
//!
//! Every phase runs its tasks in parallel rounds whose memory is reserved through the
//! [`MemoryAccountant`](everygraph_core::MemoryAccountant) first; a budget that cannot
//! hold one task is refused with [`DictionaryError::Budget`]. The buffers of the build
//! and of [`Dictionary::renumber`] that can reach 128 KiB come from the accountant's
//! [`Pages`](everygraph_core::MemoryAccountant::pages): with `everygraph_io::PAGES` a
//! freed buffer goes back to the system, so the process's anonymous memory follows the
//! ledger instead of the allocator's retention (`design/DECISIONS.md`, "The memory
//! promise covers the process"). A refused map ends the build with
//! [`DictionaryError::AllocationFailed`]. What stays on the global allocator: the
//! sections read when a dictionary is opened, the hash function's parts and the rank
//! bitmap (each allocated once and kept), the prefix index, and the writers of
//! [`MemoryScratch`], which stand for the disk.
//!
//! # The stored form, for a reader
//!
//! What follows is enough to **read** a dictionary — look keys up and ids back — from
//! its sections in another language. It does not describe how to *build* the same
//! bytes: which solution the solver finds (pilot order, eviction cost, the number of
//! recently placed buckets protected, the eviction limit, the seeds tried, the order of
//! remap entries), the part count chosen from the sample, and the byte-identity of two
//! builds are properties of this implementation, documented in its code
//! (`ids/mphf.rs`, `ids/build.rs`), not of the format. A second builder only has to
//! produce sections a reader accepts.
//!
//! **Normalized bytes** — what is hashed, stored and compared:
//!
//! - integers: 8 bytes, big-endian, of the value (unsigned columns and decimal text)
//!   or of the value with its sign bit flipped (signed columns), so that byte order is
//!   numeric order;
//! - hexadecimal: the digits as bytes, after the prefix if any, right-aligned: an odd
//!   number of digits leaves the high nibble of the first byte zero;
//! - UUID: the 16 bytes of the 32 digits;
//! - base58 (Bitcoin alphabet): the decoded bytes, one zero byte per leading `1`;
//! - fixed-size binary and bytes: the bytes as they are.
//!
//! **Hashing.** A key's hash is XXH3-128 of its normalized bytes with the dictionary's
//! seed; `hi` is the upper 64 bits, `lo` the lower. All arithmetic below is on `u64`
//! and wraps (multiplications are modulo 2^64); `mul_hi(a, b)` is the upper 64 bits of
//! the 128-bit product, `mix` the splitmix64 finalizer (`z ^= z >> 30; z *=
//! 0xBF58476D1CE4E5B9; z ^= z >> 27; z *= 0x94D049BB133111EB; z ^= z >> 31`). The hash
//! function is `PtrHash` (Groot Koerkamp, 2025) with independent parts:
//!
//! - part `q` = the top `part_bits` bits of `hi` (0 when `part_bits` is 0);
//! - bucket = `mul_hi(skew(mix(hi ^ seed_q · 0xD6E8FEB86659FD93)), buckets_q)`, with
//!   `skew(x) = mul_hi(mul_hi(x, x), (x >> 1) | 2^63) / 256 · 255 + x / 256`;
//! - slot = `mul_hi((lo ^ mix(pilot | seed_q << 8)) · 0x9E3779B97F4A7C15, slots_q)`,
//!   with `pilot` the byte of the bucket in `ids.pilots`; a slot `s ≥ n_q` is replaced
//!   by `remap[remap_base_q + s − n_q]`;
//! - the global slot is `slot_base_q + slot`.
//!
//! A part of `n` keys has `n + ⌈n/100⌉` slots and `⌈2n/7⌉` buckets (at least 1; none
//! for an empty part). The slot found must then be verified: the key stored for it is
//! compared with the key looked up.
//!
//! **Header codes.** Kind (4 bytes): `1 0 0 0` integers; `2 p d 0` hexadecimal, `p` = 1
//! with a prefix, `d` digits (1–128); `3 0 0 0` UUID; `4 w 0 0` base58 of `w` decoded
//! bytes; `5 w 0 0` fixed-size binary of `w` bytes; `6 0 0 0` bytes. Column type
//! (2 bytes): `1 b` signed integers of `b` bits, `2 b` unsigned, `3 0` utf8, `4 0`
//! binary, `5 w` fixed-size binary of `w` bytes. Any other value, a non-zero reserved
//! byte, or a version other than 1 makes the dictionary unreadable.
//!
//! # Sections
//!
//! A dictionary is a set of named byte sections ([`Dictionary::sections`],
//! [`SECTION_NAMES`]), stored by the container (#28, #35); all integers little-endian.
//! `n` is the number of keys, `w` the key width of a fixed-width kind.
//!
//! | Section | Present | Content |
//! |---|---|---|
//! | `ids.header` | always | 64 bytes: magic `EGIDDICT`; version `u32` = 1; kind (4 bytes); column type (2); order (1 = first appearance, 2 = sorted key); layout (1 = split, 2 = slot-ordered); representation (1 = hashed, 2 = dense); renumbered (0/1); 2 zero bytes; `n` `u64`; hash seed `u64`; `part_bits` `u32`; number of parts `u32` (`2^part_bits`, or 0 when dense); 16 zero bytes |
//! | `ids.parts` | hashed | one row per part: `slot_base, n, slots, bucket_base, buckets, remap_base, seed, 0`, eight `u32` |
//! | `ids.pilots` | hashed | one byte per bucket, parts one after the other |
//! | `ids.remap` | hashed | per part, `slots − n` `u32` local slots, each below the part's `n` |
//! | `ids.dense` | dense | bitmap of the keys (`u64` words; bit `k % 64` of word `k / 64` for key `k`) |
//! | `ids.slot_to_id` | split | `n` `u32`: the id of the key in each slot |
//! | `ids.keys` | split | normalized keys in id order: `w·n` bytes, or concatenated |
//! | `ids.key_offsets` | split, variable width | `n + 1` `u64` offsets into `ids.keys`, from 0 to its length |
//! | `ids.records` | slot-ordered | in slot order: `[key \| id u32]` (`w + 4` bytes) or, variable width, `[id u32 \| key]` |
//! | `ids.record_offsets` | slot-ordered, variable width | `n + 1` `u64` offsets into `ids.records`, from 0 to its length |
//! | `ids.id_to_slot` | slot-ordered | `n` `u32`: the slot of each id |
//! | `ids.order` | hashed, renumbered | `n` `u32`: graph position of each id |
//! | `ids.rank_to_id`, `ids.id_to_rank` | dense, renumbered | `n` `u32` each: the id of each rank and back (the rank is the graph position) |
//!
//! Every id, slot and rank stored is below `n`; two consecutive offsets are at most
//! [`MAX_KEY_BYTES`] apart, and in `ids.record_offsets` at least 4 and at most
//! `MAX_KEY_BYTES + 4` apart (every record holds its id); [`Dictionary::open`],
//! every look-up and [`Dictionary::renumber`] check this, and refuse a section that breaks it with
//! [`DictionaryError::Corrupt`] before allocating anything for it. The content of
//! `ids.slot_to_id` is not otherwise verified when a dictionary is opened: a wrong id
//! there makes [`Dictionary::lookup`] find nothing (it compares keys), and
//! [`Dictionary::id_of_member`] (which does not) return a wrong id below `n`.
//!
//! A forward look-up reads the hash function from memory and then, in the split
//! layout, `ids.slot_to_id` and `ids.keys` (two reads, one when the key is known to be
//! present: [`Dictionary::id_of_member`]); in the slot-ordered layout one record. A
//! reverse look-up is one read (split) or two (slot-ordered).

mod build;
mod dense;
mod dict;
mod hash;
mod kind;
mod mphf;
mod prefix;
mod scratch;
mod table;

use std::error::Error;
use std::fmt;

use everygraph_core::{InsufficientBudget, NodeId, ReadError};

pub use build::{
    Added, BuildOptions, Built, Collector, Distinct, InvalidKey, InvalidKeys, KeySource, Worker,
    build_dictionary,
};
pub use dict::{Dictionary, SECTION_NAMES};
pub use kind::{ColumnType, Key, KeyKind, MAX_FIXED_WIDTH, MAX_KEY_BYTES, OwnedKey, SAMPLE_SIZE};
pub use scratch::{MemoryScratch, Scratch};

/// How dense ids — and so graph order — are assigned when a dictionary is built.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IdOrder {
    /// Sorted for integer keys, first appearance for every other kind.
    #[default]
    Natural,
    /// In the order the keys first appear in the input (smallest position first).
    FirstAppearance,
    /// In the order of the normalized keys: numeric for integers, the lowercase text
    /// for hexadecimal and UUIDs, bytewise otherwise.
    SortedKey,
}

impl IdOrder {
    /// What [`IdOrder::Natural`] means for `kind`; other orders are kept.
    #[must_use]
    pub fn resolve(self, kind: KeyKind) -> Self {
        match (self, kind) {
            (Self::Natural, KeyKind::Integers) => Self::SortedKey,
            (Self::Natural, _) => Self::FirstAppearance,
            (order, _) => order,
        }
    }
}

/// How the arrays behind a look-up are arranged (module documentation, "Sections").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Layout {
    /// Slot to id, and keys in id order: a forward look-up reads two places, a reverse
    /// look-up one.
    #[default]
    Split,
    /// `[key | id]` records in slot order, and id to slot: a forward look-up reads one
    /// place, a reverse look-up two.
    SlotOrdered,
}

/// Why building, opening or using a dictionary failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum DictionaryError {
    /// The memory budget cannot hold even the smallest piece of the work.
    Budget(InsufficientBudget),
    /// The budget had room for a buffer of `bytes` bytes, but the system refused it.
    AllocationFailed {
        /// Bytes of the buffer.
        bytes: u64,
    },
    /// A section, a run or the input could not be read.
    Read(ReadError),
    /// The scratch storage failed.
    Scratch(std::io::Error),
    /// While integer keys were collected in a bitmap, one was outside `0..2^32`:
    /// start again with [`BuildOptions::dense`] off ([`build_dictionary`] does).
    NotDense,
    /// The input has `count` or more distinct keys, and node ids are 32 bits.
    TooManyNodes {
        /// Distinct keys counted when the limit was reached.
        count: u64,
    },
    /// Two different keys have the same 128-bit hash; [`build_dictionary`] retries with
    /// another seed.
    HashCollision,
    /// A part of the hash function could not be solved; [`build_dictionary`] retries
    /// with another seed.
    Unsolvable,
    /// Every hash seed tried met a collision or an unsolvable part (never seen; possible
    /// only with crafted input).
    NoHashFunction {
        /// Seeds tried.
        seeds: u64,
    },
    /// An id that is not below the dictionary's length.
    UnknownId(NodeId),
    /// The argument of [`Dictionary::renumber`] is not a permutation of `0..len`.
    InvalidPermutation,
    /// The operation does not apply to this dictionary.
    Unsupported(&'static str),
    /// The sections do not form a valid dictionary.
    Corrupt(&'static str),
    /// The input failed, with its own error.
    Input(Box<dyn Error + Send + Sync>),
}

impl fmt::Display for DictionaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget(e) => write!(f, "identifier dictionary: {e}"),
            Self::AllocationFailed { bytes } => write!(
                f,
                "identifier dictionary: the system could not allocate {bytes} bytes, within the memory budget"
            ),
            Self::Read(e) => write!(f, "identifier dictionary: {e}"),
            Self::Scratch(e) => write!(f, "identifier dictionary: temporary storage failed: {e}"),
            Self::NotDense => write!(f, "an integer key is outside 0..2^32"),
            Self::TooManyNodes { count } => write!(
                f,
                "the input has at least {count} distinct node identifiers; this version of Everygraph \
                 numbers nodes with 32 bits and supports at most 4294967295 (2^32 - 1) nodes per graph. \
                 Graphs with more nodes are planned in issue #114 (beyond 2^32 nodes)"
            ),
            Self::HashCollision => {
                write!(f, "two different identifiers have the same 128-bit hash")
            }
            Self::Unsolvable => write!(f, "a part of the hash function could not be solved"),
            Self::NoHashFunction { seeds } => write!(
                f,
                "no hash function found for these identifiers after {seeds} seeds (crafted input?)"
            ),
            Self::UnknownId(id) => write!(f, "node id {id} is not in the dictionary"),
            Self::InvalidPermutation => {
                write!(f, "the renumbering is not a permutation of the node ids")
            }
            Self::Unsupported(what) => write!(f, "not supported: {what}"),
            Self::Corrupt(what) => write!(f, "invalid identifier dictionary: {what}"),
            Self::Input(e) => write!(f, "reading the identifiers failed: {e}"),
        }
    }
}

impl Error for DictionaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Budget(e) => Some(e),
            Self::Read(e) => Some(e),
            Self::Scratch(e) => Some(e),
            Self::Input(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<InsufficientBudget> for DictionaryError {
    fn from(e: InsufficientBudget) -> Self {
        Self::Budget(e)
    }
}

impl From<ReadError> for DictionaryError {
    fn from(e: ReadError) -> Self {
        Self::Read(e)
    }
}

impl From<std::io::Error> for DictionaryError {
    fn from(e: std::io::Error) -> Self {
        Self::Scratch(e)
    }
}

/// The most distinct keys a dictionary takes: node ids are `u32`, and 2^32 itself is
/// refused (ADR 0003).
const MAX_NODES: u64 = (1 << 32) - 1;

fn too_many_nodes(count: u64) -> DictionaryError {
    DictionaryError::TooManyNodes { count }
}

/// Refuses a count of distinct keys that reaches 2^32.
fn check_node_count(count: u64) -> Result<(), DictionaryError> {
    if count > MAX_NODES {
        Err(too_many_nodes(count))
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;
