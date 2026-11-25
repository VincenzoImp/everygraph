// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The identifier dictionary: the keys of the input (addresses, user names, URLs,
//! integers) and the dense node ids `0..n` the engine works with, in both directions
//! (DESIGN D8, ADR 0008; issue #34).
//!
//! # Use
//!
//! [`build_dictionary`] reads a [`KeySource`] (any chunked, re-readable input) and
//! returns a [`Dictionary`]: [`Dictionary::lookup`] maps a key to its id,
//! [`Dictionary::key`] an id back to its key, in the type of the input column
//! ([`ColumnType`], `docs/api-sketch.md` § 2.2). A pipeline that reads its input
//! itself drives a [`Collector`] directly: one [`Worker`] per thread, then
//! [`Collector::finish`] and [`Distinct::build`].
//!
//! # Keys
//!
//! The kind of the keys is chosen from the first [`SAMPLE_SIZE`] keys of the input
//! ([`KeyKind::detect`]) and checked on every key; a key that does not fit restarts the
//! pass with a wider kind, so the choice depends on the input only. Integers are stored
//! as 8 bytes, hexadecimal text, UUIDs and base58 as their decoded bytes, keys of one
//! constant length as they are, all at a fixed width with no offsets; anything else as
//! bytes with offsets. Hexadecimal and UUID keys are matched in any case and given back
//! in lowercase with the prefix the input used.
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
//! ADR 0003); ids beyond 32 bits are issue #114.
//!
//! # How a dictionary is built
//!
//! 1. **Distinct keys.** Each worker puts keys into its own hash table (key, smallest
//!    position seen). When the table cannot grow within the memory budget, it is
//!    written to the [`Scratch`] as a *run*, grouped by the top 12 bits of the key hash
//!    (4,096 fine partitions), and emptied. One key in 1,024 (by hash) is sampled.
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
//! hold one task is refused with [`DictionaryError::Budget`] before anything is
//! allocated.
//!
//! # Hashing
//!
//! A key's hash is XXH3-128 of its normalized bytes with the dictionary's seed; `hi`
//! is the upper 64 bits, `lo` the lower. The hash function is `PtrHash` (Groot
//! Koerkamp, 2025) with independent parts:
//!
//! - part `q` = the top `part_bits` bits of `hi`;
//! - bucket = `mul_hi(skew(mix(hi ^ seed_q·0xD6E8FEB86659FD93)), buckets_q)`, with
//!   `mix` the splitmix64 finalizer and `skew(x) = mul_hi(mul_hi(x, x), (x >> 1) | 2^63)
//!   / 256 · 255 + x / 256`;
//! - slot = `mul_hi((lo ^ mix(pilot | seed_q << 8)) · 0x9E3779B97F4A7C15, slots_q)`,
//!   with `pilot` the byte of the bucket in `ids.pilots`; a slot `s ≥ n_q` is replaced
//!   by `remap[remap_base_q + s − n_q]`;
//! - the global slot is `slot_base_q + slot`.
//!
//! `mul_hi(a, b)` is the upper 64 bits of the 128-bit product. A part of `n` keys has
//! `n + ⌈n/100⌉` slots and `⌈2n/7⌉` buckets (at least 1). About 2.6 bits per key.
//!
//! # Sections
//!
//! A dictionary is a set of named byte sections ([`Dictionary::sections`],
//! [`SECTION_NAMES`]), stored by the container (#28, #35); all integers little-endian.
//! `n` is the number of keys, `w` the key width of a fixed-width kind.
//!
//! | Section | Present | Content |
//! |---|---|---|
//! | `ids.header` | always | 64 bytes: magic `EGIDDICT`; version `u32` = 1; kind (4 bytes); column type (2); order (1 = first appearance, 2 = sorted key); layout (1 = split, 2 = slot-ordered); representation (1 = hashed, 2 = dense); renumbered (0/1); 2 zero bytes; `n` `u64`; hash seed `u64`; `part_bits` `u32`; number of parts `u32`; 16 zero bytes |
//! | `ids.parts` | hashed | one row per part: `slot_base, n, slots, bucket_base, buckets, remap_base, seed, 0`, eight `u32` |
//! | `ids.pilots` | hashed | one byte per bucket, parts one after the other |
//! | `ids.remap` | hashed | per part, `slots − n` `u32` local slots |
//! | `ids.dense` | dense | bitmap of the keys (`u64` words; bit `k` of word `k/64` for key `k`) |
//! | `ids.slot_to_id` | split | `n` `u32`: the id of the key in each slot |
//! | `ids.keys` | split | normalized keys in id order: `w·n` bytes, or concatenated |
//! | `ids.key_offsets` | split, variable width | `n + 1` `u64` offsets into `ids.keys` |
//! | `ids.records` | slot-ordered | in slot order: `[key \| id u32]` (`w + 4` bytes) or, variable width, `[id u32 \| key]` |
//! | `ids.record_offsets` | slot-ordered, variable width | `n + 1` `u64` offsets into `ids.records` |
//! | `ids.id_to_slot` | slot-ordered | `n` `u32`: the slot of each id |
//! | `ids.order` | hashed, renumbered | `n` `u32`: graph position of each id |
//! | `ids.rank_to_id`, `ids.id_to_rank` | dense, renumbered | `n` `u32` each: the id of each rank and back (the rank is the graph position) |
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

pub use build::{BuildOptions, Collector, Distinct, KeySource, Worker, build_dictionary};
pub use dict::{Dictionary, SECTION_NAMES};
pub use kind::{ColumnType, HexPrefix, Key, KeyKind, MAX_FIXED_WIDTH, OwnedKey, SAMPLE_SIZE};
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
            (Self::Natural, KeyKind::Integers { .. }) => Self::SortedKey,
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
    /// A section, a run or the input could not be read.
    Read(ReadError),
    /// The scratch storage failed.
    Scratch(std::io::Error),
    /// A key does not fit the kind chosen from the sample; [`build_dictionary`]
    /// handles this by widening the kind.
    KindMismatch {
        /// Where the key was seen.
        position: u64,
        /// The key, shortened for the message.
        key: String,
    },
    /// The input has `count` or more distinct keys, and node ids are 32 bits.
    TooManyNodes {
        /// Distinct keys counted when the limit was reached.
        count: u64,
    },
    /// Two different keys have the same 128-bit hash; [`build_dictionary`] retries with
    /// another seed.
    HashCollision,
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

impl DictionaryError {
    fn mismatch(position: u64, key: Key<'_>) -> Self {
        let key = match key {
            Key::Int(v) => v.to_string(),
            Key::UInt(v) => v.to_string(),
            Key::Bytes(b) => String::from_utf8_lossy(&b[..b.len().min(64)]).into_owned(),
        };
        Self::KindMismatch { position, key }
    }
}

impl fmt::Display for DictionaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget(e) => write!(f, "identifier dictionary: {e}"),
            Self::Read(e) => write!(f, "identifier dictionary: {e}"),
            Self::Scratch(e) => write!(f, "identifier dictionary: temporary storage failed: {e}"),
            Self::KindMismatch { position, key } => {
                write!(
                    f,
                    "the identifier {key:?} at position {position} does not have the form of the others"
                )
            }
            Self::TooManyNodes { count } => write!(
                f,
                "the input has at least {count} distinct node identifiers; this version of Everygraph \
                 numbers nodes with 32 bits and supports at most 4294967295 (2^32 - 1) nodes per graph. \
                 Graphs with more nodes are planned in issue #114 (beyond 2^32 nodes)"
            ),
            Self::HashCollision => {
                write!(f, "two different identifiers have the same 128-bit hash")
            }
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
