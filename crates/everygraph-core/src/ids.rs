// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Node ids and edge offsets, and the only sanctioned conversions between them and
//! `usize`.
//!
//! Three integer widths meet in a graph engine, and mixing them silently is how a kernel
//! that passes on x86-64 corrupts data on wasm32:
//!
//! - a **node id** is 32 bits (u64 targets cost +71% to +87% on `PageRank` in the lab; D3);
//! - an **edge offset** is 64 bits wherever it is stored or computed, because a graph
//!   with more than 2^32 adjacency entries is ordinary;
//! - an **index into memory** is `usize`, which is 32 bits in the browser.
//!
//! Going up (`u32` → `usize` → `u64`) never loses information and is infallible here.
//! Going down is checked and returns an error instead of truncating.

use std::error::Error;
use std::fmt;

use crate::widen;

/// Dense identifier of a node: the nodes of a graph with `n` nodes are `0..n`.
///
/// It is a plain `u32` in memory and on disk. Use [`NodeId::index`] to index an array.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct NodeId(u32);

impl NodeId {
    /// Wraps a raw id.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw id.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The id as an index into an in-memory array. Never truncates: `usize` is at least
    /// 32 bits wide on every supported target.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl From<u32> for NodeId {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

impl From<NodeId> for u32 {
    fn from(id: NodeId) -> Self {
        id.0
    }
}

impl TryFrom<u64> for NodeId {
    type Error = NodeIdOverflow;

    /// Fails if `value` is larger than `u32::MAX`.
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        u32::try_from(value)
            .map(Self)
            .map_err(|_| NodeIdOverflow { value })
    }
}

impl TryFrom<usize> for NodeId {
    type Error = NodeIdOverflow;

    /// Fails if `index` is larger than `u32::MAX` (possible on 64-bit targets only).
    fn try_from(index: usize) -> Result<Self, Self::Error> {
        Self::try_from(widen(index))
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Position in an adjacency array: the number of adjacency entries that precede it.
///
/// It is a `u64` wherever it is stored or computed. Convert it with
/// [`EdgeOffset::to_index`] at the point where memory is indexed, and nowhere earlier:
/// on a 32-bit target that conversion is the check that the data fits in memory at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct EdgeOffset(u64);

impl EdgeOffset {
    /// Wraps a raw offset.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw offset.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The offset as an index into an in-memory array.
    ///
    /// # Errors
    ///
    /// Fails if the offset is larger than `usize::MAX`, which can only happen where
    /// `usize` is 32 bits wide (wasm32, i686).
    pub fn to_index(self) -> Result<usize, OffsetOverflow> {
        usize::try_from(self.0).map_err(|_| OffsetOverflow { value: self.0 })
    }
}

impl From<u64> for EdgeOffset {
    fn from(raw: u64) -> Self {
        Self(raw)
    }
}

impl From<usize> for EdgeOffset {
    /// Never truncates: `usize` is at most 64 bits wide on every supported target.
    fn from(index: usize) -> Self {
        Self(widen(index))
    }
}

impl From<EdgeOffset> for u64 {
    fn from(offset: EdgeOffset) -> Self {
        offset.0
    }
}

impl TryFrom<EdgeOffset> for usize {
    type Error = OffsetOverflow;

    /// Same as [`EdgeOffset::to_index`].
    fn try_from(offset: EdgeOffset) -> Result<Self, Self::Error> {
        offset.to_index()
    }
}

impl fmt::Display for EdgeOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A value that was meant to become a [`NodeId`] does not fit in 32 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeIdOverflow {
    value: u64,
}

impl NodeIdOverflow {
    /// The value that did not fit.
    #[must_use]
    pub const fn value(&self) -> u64 {
        self.value
    }
}

impl fmt::Display for NodeIdOverflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "node id {} does not fit in 32 bits (the largest node id is {})",
            self.value,
            u32::MAX
        )
    }
}

impl Error for NodeIdOverflow {}

/// An [`EdgeOffset`] does not fit in this platform's `usize`, so the position it names
/// cannot be in memory here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OffsetOverflow {
    value: u64,
}

impl OffsetOverflow {
    /// The offset that did not fit.
    #[must_use]
    pub const fn value(&self) -> u64 {
        self.value
    }
}

impl fmt::Display for OffsetOverflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "edge offset {} cannot be addressed in memory on this platform ({}-bit addresses)",
            self.value,
            usize::BITS
        )
    }
}

impl Error for OffsetOverflow {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_is_a_plain_u32() {
        assert_eq!(size_of::<NodeId>(), 4);
        assert_eq!(align_of::<NodeId>(), align_of::<u32>());
        assert_eq!(NodeId::default(), NodeId::new(0));
        assert_eq!(NodeId::new(7).get(), 7);
        assert_eq!(u32::from(NodeId::from(7_u32)), 7);
        assert_eq!(NodeId::new(7).to_string(), "7");
    }

    #[test]
    fn node_id_index_is_lossless_at_the_top_of_the_range() {
        let top = NodeId::new(u32::MAX);
        assert_eq!(widen(top.index()), u64::from(u32::MAX));
        assert_eq!(NodeId::try_from(top.index()), Ok(top));
    }

    #[test]
    fn node_id_from_u64_is_checked() {
        assert_eq!(NodeId::try_from(0_u64), Ok(NodeId::new(0)));
        assert_eq!(
            NodeId::try_from(u64::from(u32::MAX)),
            Ok(NodeId::new(u32::MAX))
        );

        let first_too_large = u64::from(u32::MAX) + 1;
        let err = NodeId::try_from(first_too_large).unwrap_err();
        assert_eq!(err.value(), first_too_large);
        assert_eq!(NodeId::try_from(u64::MAX).unwrap_err().value(), u64::MAX);
        assert_eq!(
            err.to_string(),
            "node id 4294967296 does not fit in 32 bits (the largest node id is 4294967295)"
        );
    }

    #[test]
    fn node_id_from_usize_is_checked() {
        assert_eq!(NodeId::try_from(41_usize), Ok(NodeId::new(41)));
    }

    /// Only a 64-bit `usize` can hold an index that is not a node id.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn node_id_from_usize_rejects_indexes_above_u32() {
        let index = usize::try_from(u64::from(u32::MAX) + 1).unwrap();
        assert_eq!(NodeId::try_from(index).unwrap_err().value(), widen(index));
    }

    #[test]
    fn node_ids_order_like_their_raw_values() {
        let mut ids = [NodeId::new(3), NodeId::new(u32::MAX), NodeId::new(0)];
        ids.sort_unstable();
        assert_eq!(ids.map(NodeId::get), [0, 3, u32::MAX]);
    }

    #[test]
    fn edge_offset_is_a_plain_u64() {
        assert_eq!(size_of::<EdgeOffset>(), 8);
        assert_eq!(EdgeOffset::default(), EdgeOffset::new(0));
        assert_eq!(EdgeOffset::new(1 << 40).get(), 1 << 40);
        assert_eq!(u64::from(EdgeOffset::from(9_u64)), 9);
        assert_eq!(EdgeOffset::new(1 << 40).to_string(), "1099511627776");
    }

    #[test]
    fn edge_offset_round_trips_every_usize() {
        for index in [0, 1, usize::MAX / 2, usize::MAX] {
            let offset = EdgeOffset::from(index);
            assert_eq!(offset.get(), widen(index));
            assert_eq!(offset.to_index(), Ok(index));
            assert_eq!(usize::try_from(offset), Ok(index));
        }
    }

    /// On a 64-bit target every offset is an index.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn edge_offset_always_fits_a_64_bit_usize() {
        assert_eq!(EdgeOffset::new(u64::MAX).to_index(), Ok(usize::MAX));
    }

    /// The case the type exists for: 2^32 adjacency entries on a 32-bit target. On the
    /// development machine this test runs only under Miri with
    /// `--target i686-unknown-linux-gnu`; nothing runs tests on wasm32 yet.
    #[cfg(target_pointer_width = "32")]
    #[test]
    fn edge_offset_above_4_gib_is_an_error_on_32_bit_targets() {
        let first_too_large = widen(usize::MAX) + 1;
        let err = EdgeOffset::new(first_too_large).to_index().unwrap_err();
        assert_eq!(err.value(), first_too_large);
        assert_eq!(
            err.to_string(),
            "edge offset 4294967296 cannot be addressed in memory on this platform (32-bit addresses)"
        );
        assert!(usize::try_from(EdgeOffset::new(u64::MAX)).is_err());
    }
}
