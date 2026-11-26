// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`Column`]: a vector reserved against the memory budget; and parallel prefix sums.

use std::fmt;
use std::num::NonZeroUsize;
use std::ops::{Deref, DerefMut};

use everygraph_core::{MAX_ALLOCATION_BYTES, Memory, MemoryError, MemoryKind, Reservation, par};

/// Elements per task when a column is made, summed or scanned.
pub(crate) const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 14).expect("non-zero");

/// One value per node, per community or per entry, held with the reservation that pays
/// for it.
pub struct Column<T> {
    values: Vec<T>,
    reservation: Reservation,
}

impl<T: Send> Column<T> {
    /// `[f(0), …, f(len - 1)]`, made in parallel after its size is reserved.
    ///
    /// A column is one allocation, not segments (DESIGN D2 asks for segments: #m4-14).
    /// On a 32-bit target, where one allocation above about 2 GiB fails, a column above
    /// [`MAX_ALLOCATION_BYTES`] is refused instead.
    pub(crate) fn new(
        memory: &Memory,
        purpose: &'static str,
        len: usize,
        f: impl Fn(usize) -> T + Send + Sync,
    ) -> Result<Self, MemoryError> {
        let bytes = (len as u64).saturating_mul(size_of::<T>() as u64);
        if usize::BITS < 64 && bytes > MAX_ALLOCATION_BYTES {
            return Err(MemoryError::AllocationFailed { purpose, bytes });
        }
        let reservation = memory.reserve(purpose, MemoryKind::Anonymous, bytes)?;
        Ok(Self {
            values: par::collect(len, GRAIN, f),
            reservation,
        })
    }
}

impl<T> Column<T> {
    /// The column of `f` of each value. For values of the same size (atomics into
    /// their integers) the vector is reused in place, and so is the reservation.
    pub(crate) fn map<U>(self, f: impl FnMut(T) -> U) -> Column<U> {
        debug_assert_eq!(size_of::<T>(), size_of::<U>());
        Column {
            values: self.values.into_iter().map(f).collect(),
            reservation: self.reservation,
        }
    }
}

impl<T> Deref for Column<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.values
    }
}

impl<T> DerefMut for Column<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.values
    }
}

impl<T> fmt::Debug for Column<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Column")
            .field("len", &self.values.len())
            .finish_non_exhaustive()
    }
}

/// `[0, c(0), c(0) + c(1), …]`: the `len + 1` offsets of runs of `c(i)` elements, by a
/// parallel prefix sum. `count` is called twice for each `i`.
pub(crate) fn offsets(
    memory: &Memory,
    purpose: &'static str,
    len: usize,
    count: impl Fn(usize) -> u64 + Send + Sync,
) -> Result<Column<u64>, MemoryError> {
    let sums = par::map_ranges(len, GRAIN, |r| r.map(&count).sum::<u64>());
    let starts: Vec<u64> = sums
        .iter()
        .scan(0, |at, &s| {
            let start = *at;
            *at += s;
            Some(start)
        })
        .collect();
    let mut out = Column::new(memory, purpose, len + 1, |_| 0)?;
    out[len] = sums.iter().sum();
    par::for_each_chunk_mut(&mut out[..len], GRAIN, |first, chunk| {
        let mut at = starts[first / GRAIN];
        for (i, o) in (first..).zip(chunk) {
            *o = at;
            at += count(i);
        }
    });
    Ok(out)
}
