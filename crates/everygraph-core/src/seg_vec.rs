// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`SegVec`]: a vector made of fixed-size segments, every byte of it reserved.

use std::fmt;
use std::num::NonZeroUsize;
use std::ops::{Index, IndexMut, Range};

use crate::memory::{Memory, MemoryError, MemoryKind, Reservation};
use crate::{par, widen};

/// The largest single allocation the engine makes, in bytes: 1 GiB.
///
/// wasm32 refuses any single allocation above 2 GiB (DESIGN D2; the lab hit it at
/// 275 M edges), and a 1 GiB ceiling leaves room for the allocator's own rounding.
pub const MAX_ALLOCATION_BYTES: u64 = 1 << 30;

/// Smallest capacity a growing vector allocates, in elements.
const MIN_GROWTH: usize = 4;

/// A vector of `T` stored as segments of `2^k` elements, each one allocation of at most
/// [`MAX_ALLOCATION_BYTES`], with every byte of capacity reserved against a [`Memory`]
/// budget before it is allocated.
///
/// **Accounting.** The reservation always equals the capacity in bytes. Growth reserves
/// before it allocates — including, when the last segment is reallocated, the moment
/// when the old and the new buffer both exist — and a growth that does not fit is
/// refused before anything is allocated. There is no `Clone` (an untracked copy) and no
/// access to the segments as `Vec`s (untracked growth): [`try_clone`](Self::try_clone)
/// reserves, and segments are lent out as slices. Dropping the vector releases its
/// reservation.
///
/// **Layout.** Element `i` lives in segment `i >> k` at offset `i & (2^k − 1)`. Every
/// segment but the last holds exactly `2^k` elements. By default `k` is the largest
/// that keeps a segment within [`MAX_ALLOCATION_BYTES`] (`2^28` elements of a 4-byte
/// type), so arrays below 1 GiB are a single slice: [`as_slice`](Self::as_slice) gives
/// it to code that wants one.
///
/// **Hot loops** iterate over slices: [`segments`](Self::segments),
/// [`range_slices`](Self::range_slices) (the parts of an index range, usually one), and
/// [`for_each_chunk_mut`](Self::for_each_chunk_mut) for parallel writes. Inside a slice
/// the compiler removes bounds checks as for any slice. Indexing (`v[i]`) costs what a
/// slice index costs within the first segment; beyond it, a shift, a mask, a load of
/// the segment's address and two bounds checks. Random access spread over many
/// segments is much slower than over one (1.7–1.8× in `PageRank` on orkut with segments
/// of 2^16 elements, exploratory): arrays of up to 1 GiB are one segment.
///
/// A result must not depend on where segments end: their length depends on
/// `size_of::<T>()`, and code that reduces per segment would change its answer with it
/// (DESIGN D14). Chunks of [`for_each_chunk_mut`](Self::for_each_chunk_mut) and of
/// [`par`] are placed by the grain only.
pub struct SegVec<T> {
    // Dropped before the reservation: memory is freed before it is given back.
    segments: Vec<Vec<T>>,
    len: usize,
    bits: u32,
    reservation: Reservation,
}

impl<T> SegVec<T> {
    /// `k` of the default layout: the largest segment within [`MAX_ALLOCATION_BYTES`].
    /// Evaluating it rejects, at compile time, a zero-sized `T` and one larger than a
    /// segment.
    const MAX_BITS: u32 = {
        let size = size_of::<T>() as u64;
        assert!(size > 0, "SegVec does not hold zero-sized types");
        assert!(
            size <= MAX_ALLOCATION_BYTES,
            "an element is larger than a segment"
        );
        (MAX_ALLOCATION_BYTES / size).ilog2()
    };

    /// An empty vector, allocating nothing, whose memory will be reserved for `purpose`.
    pub fn new(memory: &Memory, purpose: &'static str) -> Self {
        Self::with_segment_bits(memory, purpose, Self::MAX_BITS)
    }

    /// An empty vector with segments of `2^bits` elements. Smaller segments than the
    /// default are for tests, and for callers that want memory to grow in smaller
    /// steps.
    ///
    /// # Panics
    ///
    /// If a segment of `2^bits` elements would exceed [`MAX_ALLOCATION_BYTES`].
    pub fn with_segment_bits(memory: &Memory, purpose: &'static str, bits: u32) -> Self {
        assert!(
            bits <= Self::MAX_BITS,
            "segments of 2^{bits} elements of {} bytes exceed MAX_ALLOCATION_BYTES",
            size_of::<T>()
        );
        Self {
            segments: Vec::new(),
            len: 0,
            bits,
            reservation: Reservation::empty(memory, purpose, MemoryKind::Anonymous),
        }
    }

    /// Bytes that `len` elements take, computed in `u64` so that a planner can ask
    /// about sizes this target cannot address. `None` if it does not fit in a `u64`.
    #[must_use]
    pub const fn bytes_for(len: u64) -> Option<u64> {
        len.checked_mul(size_of::<T>() as u64)
    }

    /// Number of elements.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether there are no elements.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Elements that fit without allocating.
    #[must_use]
    pub fn capacity(&self) -> usize {
        match self.segments.len() {
            0 => 0,
            n => ((n - 1) << self.bits) + self.segments[n - 1].capacity(),
        }
    }

    /// Bytes reserved, which is the capacity in bytes.
    #[must_use]
    pub const fn reserved_bytes(&self) -> u64 {
        self.reservation.bytes()
    }

    /// What the memory is reserved for.
    #[must_use]
    pub const fn purpose(&self) -> &'static str {
        self.reservation.purpose()
    }

    /// Elements per segment: a power of two.
    #[must_use]
    pub const fn segment_len(&self) -> usize {
        1 << self.bits
    }

    /// The slices holding the elements, in order: every one but the last is
    /// [`segment_len`](Self::segment_len) long, and none is empty.
    pub fn segments(&self) -> impl ExactSizeIterator<Item = &[T]> + '_ {
        self.segments[..self.used_segments()]
            .iter()
            .map(Vec::as_slice)
    }

    /// The slices holding the elements, mutably, in order.
    pub fn segments_mut(&mut self) -> impl ExactSizeIterator<Item = &mut [T]> + '_ {
        let used = self.used_segments();
        self.segments[..used].iter_mut().map(Vec::as_mut_slice)
    }

    /// The elements as one slice, when they are in a single segment (always the case
    /// below [`segment_len`](Self::segment_len) elements).
    #[must_use]
    pub fn as_slice(&self) -> Option<&[T]> {
        match self.used_segments() {
            0 => Some(&[]),
            1 => Some(&self.segments[0]),
            _ => None,
        }
    }

    /// The elements as one mutable slice, when they are in a single segment.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> Option<&mut [T]> {
        match self.used_segments() {
            0 => Some(&mut []),
            1 => Some(&mut self.segments[0]),
            _ => None,
        }
    }

    /// The element at `index`, if there is one.
    #[must_use]
    #[inline]
    pub fn get(&self, index: usize) -> Option<&T> {
        if let Some(first) = self.segments.first() {
            if index < first.len() {
                return first.get(index);
            }
        }
        self.segments
            .get(index >> self.bits)?
            .get(index & self.mask())
    }

    /// The element at `index`, mutably, if there is one.
    #[must_use]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        let mask = self.mask();
        self.segments
            .get_mut(index >> self.bits)?
            .get_mut(index & mask)
    }

    /// The elements in order.
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.segments().flatten()
    }

    /// The elements of `range` as consecutive slices: one, unless the range crosses
    /// the end of a segment.
    ///
    /// # Panics
    ///
    /// If the range is decreasing or ends beyond [`len`](Self::len), as slice indexing
    /// does.
    pub fn range_slices(&self, range: Range<usize>) -> impl Iterator<Item = &[T]> + '_ {
        assert!(
            range.start <= range.end && range.end <= self.len,
            "range {range:?} out of bounds for a SegVec of length {}",
            self.len
        );
        let Range { mut start, end } = range;
        std::iter::from_fn(move || {
            if start == end {
                return None;
            }
            let segment = &self.segments[start >> self.bits];
            let offset = start & self.mask();
            let take = (end - start).min(segment.len() - offset);
            start += take;
            Some(&segment[offset..offset + take])
        })
    }

    /// Calls `f(start, chunk)` for chunks of `grain` elements, possibly in parallel,
    /// where `start` is the index of the chunk's first element: chunk `i` covers
    /// `i * grain .. min((i + 1) * grain, len)`, exactly as
    /// [`par::for_each_chunk_mut`] on a flat slice.
    ///
    /// # Panics
    ///
    /// If `grain` does not divide [`segment_len`](Self::segment_len) (it must be a power
    /// of two no larger), so that no chunk straddles two segments. This is checked
    /// whatever the length, so that a wrong grain fails on small inputs too.
    pub fn for_each_chunk_mut<F>(&mut self, grain: NonZeroUsize, f: F)
    where
        T: Send,
        F: Fn(usize, &mut [T]) + Send + Sync,
    {
        let segment_len = self.segment_len();
        assert!(
            segment_len % grain == 0,
            "a grain of {grain} does not divide segments of {segment_len} elements"
        );
        for (index, segment) in self.segments_mut().enumerate() {
            let base = index * segment_len;
            par::for_each_chunk_mut(segment, grain, |start, chunk| f(base + start, chunk));
        }
    }

    /// Reserves room for at least `additional` more elements, exactly as many as
    /// asked.
    ///
    /// # Errors
    ///
    /// [`MemoryError::Budget`] if the budget does not have room, before anything is
    /// allocated; [`MemoryError::AllocationFailed`] if the allocator refused. The
    /// vector's elements are unchanged either way.
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), MemoryError> {
        let target = self
            .len
            .checked_add(additional)
            .ok_or_else(|| self.unaddressable())?;
        self.grow_to(target)
    }

    /// Appends `value`, growing as a `Vec` would (doubling within the first segment,
    /// then one segment at a time).
    ///
    /// # Errors
    ///
    /// As [`try_reserve`](Self::try_reserve); `value` is dropped then.
    pub fn try_push(&mut self, value: T) -> Result<(), MemoryError> {
        let needed = self
            .len
            .checked_add(1)
            .ok_or_else(|| self.unaddressable())?;
        self.grow_to(self.amortized(needed))?;
        let index = self.len;
        self.segments[index >> self.bits].push(value);
        self.len += 1;
        Ok(())
    }

    /// Shortens the vector to `len` elements, keeping the capacity (and its
    /// reservation). Nothing happens if it is not longer.
    pub fn truncate(&mut self, len: usize) {
        if len >= self.len {
            return;
        }
        let bits = self.bits;
        for (index, segment) in self.segments.iter_mut().enumerate() {
            segment.truncate(len.saturating_sub(index << bits));
        }
        self.len = len;
    }

    /// Removes every element, keeping the capacity.
    pub fn clear(&mut self) {
        self.truncate(0);
    }

    /// Frees the capacity beyond the length, and its reservation. Shrinking the last
    /// segment may move it, so it is shrunk only if the budget has room for the
    /// smaller copy next to the old one; otherwise that segment keeps its capacity.
    pub fn shrink_to_fit(&mut self) {
        let used = self.used_segments();
        for segment in self.segments.drain(used..) {
            self.reservation.shrink(Self::bytes_of(segment.capacity()));
        }
        if let Some(last) = self.segments.last() {
            if last.len() < last.capacity() {
                let _ = self.resize_last(last.len());
            }
        }
    }

    /// Bytes of `elements` elements; only for counts that fit in memory, whose size
    /// cannot overflow a `u64`.
    const fn bytes_of(elements: usize) -> u64 {
        widen(elements) * (size_of::<T>() as u64)
    }

    const fn mask(&self) -> usize {
        self.segment_len() - 1
    }

    fn used_segments(&self) -> usize {
        self.len.div_ceil(self.segment_len())
    }

    fn unaddressable(&self) -> MemoryError {
        MemoryError::AllocationFailed {
            purpose: self.purpose(),
            bytes: u64::MAX,
        }
    }

    /// The capacity to grow to when `needed` elements must fit and more will follow.
    fn amortized(&self, needed: usize) -> usize {
        let segment_len = self.segment_len();
        let capacity = self.capacity();
        if needed <= capacity {
            capacity
        } else if needed <= segment_len {
            needed
                .max(capacity.saturating_mul(2))
                .max(MIN_GROWTH)
                .min(segment_len)
        } else {
            // Round up to whole segments: one more segment at a time, not twice the
            // memory.
            needed
                .checked_next_multiple_of(segment_len)
                .unwrap_or(needed)
        }
    }

    /// Makes the capacity exactly `target` elements if it is less, reserving first.
    fn grow_to(&mut self, target: usize) -> Result<(), MemoryError> {
        let capacity = self.capacity();
        if target <= capacity {
            return Ok(());
        }
        let segment_len = self.segment_len();
        let Some(target_bytes) = Self::bytes_for(widen(target)) else {
            return Err(self.unaddressable());
        };
        let growth = target_bytes - Self::bytes_of(capacity);

        // The last segment, if it is not full-sized, is reallocated: its old buffer
        // exists next to the new one until the data has moved, so that is reserved
        // too. A refusal here comes before anything is allocated.
        let base = self.segments.len().saturating_sub(1) << self.bits;
        let reallocated = self
            .segments
            .last()
            .map(Vec::capacity)
            .filter(|&last| last < segment_len);
        let transient = reallocated.map_or(0, Self::bytes_of);
        let Some(planned) = growth.checked_add(transient) else {
            return Err(self.unaddressable());
        };
        self.reservation.try_grow(planned)?;
        let mut pending = planned;

        let mut covered = capacity;
        if let (Some(old), Some(last)) = (reallocated, self.segments.last_mut()) {
            let new = (target - base).min(segment_len);
            if last.try_reserve_exact(new - last.len()).is_err() {
                self.reservation.shrink(pending);
                return Err(self.allocation_failed(new));
            }
            // The old buffer is gone; what is left pending is the new segments.
            pending -= Self::bytes_of(new - old) + transient;
            self.reservation.shrink(transient);
            covered = base + new;
        }
        while covered < target {
            let size = (target - covered).min(segment_len);
            let mut segment = Vec::new();
            if self.segments.try_reserve(1).is_err() || segment.try_reserve_exact(size).is_err() {
                self.reservation.shrink(pending);
                return Err(self.allocation_failed(size));
            }
            self.segments.push(segment);
            pending -= Self::bytes_of(size);
            covered += size;
        }
        debug_assert_eq!(pending, 0);
        Ok(())
    }

    /// Reallocates the last segment to `capacity` elements (at least its length),
    /// reserving the new buffer before the old one is released.
    fn resize_last(&mut self, capacity: usize) -> Result<(), MemoryError> {
        let new_bytes = Self::bytes_of(capacity);
        self.reservation.try_grow(new_bytes)?;
        let Some(last) = self.segments.last_mut() else {
            self.reservation.shrink(new_bytes);
            return Ok(());
        };
        let old_bytes = Self::bytes_of(last.capacity());
        if capacity >= last.capacity() {
            if last.try_reserve_exact(capacity - last.len()).is_err() {
                self.reservation.shrink(new_bytes);
                return Err(self.allocation_failed(capacity));
            }
        } else {
            last.shrink_to(capacity);
        }
        self.reservation.shrink(old_bytes);
        Ok(())
    }

    fn allocation_failed(&self, elements: usize) -> MemoryError {
        MemoryError::AllocationFailed {
            purpose: self.purpose(),
            bytes: Self::bytes_of(elements),
        }
    }
}

impl<T: Clone> SegVec<T> {
    /// A vector of `len` copies of `value`. The whole size is reserved before the first
    /// segment is allocated, so a refusal allocates nothing.
    ///
    /// # Errors
    ///
    /// [`MemoryError::Budget`] if `len` elements do not fit in the budget (`needed` is
    /// their size in bytes); [`MemoryError::AllocationFailed`] if the allocator
    /// refused.
    pub fn try_with_len(
        memory: &Memory,
        purpose: &'static str,
        len: usize,
        value: T,
    ) -> Result<Self, MemoryError> {
        let mut vec = Self::new(memory, purpose);
        vec.try_resize(len, value)?;
        Ok(vec)
    }

    /// Makes the length `len`, appending copies of `value` or truncating.
    ///
    /// # Errors
    ///
    /// As [`try_reserve`](Self::try_reserve), with the elements unchanged.
    pub fn try_resize(&mut self, len: usize, value: T) -> Result<(), MemoryError> {
        if len <= self.len {
            self.truncate(len);
            return Ok(());
        }
        self.grow_to(len)?;
        let bits = self.bits;
        for (index, segment) in self.segments.iter_mut().enumerate() {
            let wanted = len.saturating_sub(index << bits).min(1 << bits);
            if segment.len() < wanted {
                segment.resize(wanted, value.clone());
            }
        }
        self.len = len;
        Ok(())
    }

    /// Appends copies of `values`.
    ///
    /// # Errors
    ///
    /// As [`try_push`](Self::try_push), with the elements unchanged.
    pub fn try_extend_from_slice(&mut self, values: &[T]) -> Result<(), MemoryError> {
        let needed = self
            .len
            .checked_add(values.len())
            .ok_or_else(|| self.unaddressable())?;
        self.grow_to(self.amortized(needed))?;
        let mut rest = values;
        while !rest.is_empty() {
            let segment = &mut self.segments[self.len >> self.bits];
            let (now, later) = rest.split_at(rest.len().min(segment.capacity() - segment.len()));
            segment.extend_from_slice(now);
            self.len += now.len();
            rest = later;
        }
        Ok(())
    }

    /// A copy, reserved for the same purpose with exactly the length as capacity.
    ///
    /// # Errors
    ///
    /// As [`try_reserve`](Self::try_reserve).
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        let mut copy =
            Self::with_segment_bits(self.reservation.memory(), self.purpose(), self.bits);
        copy.grow_to(self.len)?;
        for segment in self.segments() {
            copy.try_extend_from_slice(segment)?;
        }
        Ok(copy)
    }

    /// Sets every element to `value`.
    pub fn fill(&mut self, value: T) {
        for segment in self.segments_mut() {
            segment.fill(value.clone());
        }
    }
}

impl<T> Index<usize> for SegVec<T> {
    type Output = T;

    #[inline]
    fn index(&self, index: usize) -> &T {
        // The first segment is checked on its own: in a loop the compiler keeps its
        // address in a register, and an array of one segment then costs what a slice
        // costs (issue #31).
        if let Some(first) = self.segments.first() {
            if index < first.len() {
                return &first[index];
            }
        }
        &self.segments[index >> self.bits][index & self.mask()]
    }
}

impl<T> IndexMut<usize> for SegVec<T> {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut T {
        let mask = self.mask();
        &mut self.segments[index >> self.bits][index & mask]
    }
}

impl<T> fmt::Debug for SegVec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SegVec")
            .field("purpose", &self.purpose())
            .field("len", &self.len)
            .field("segment_len", &self.segment_len())
            .field("reserved_bytes", &self.reserved_bytes())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::memory::{Budget, InsufficientBudget};

    fn budget(limit: u64) -> (Arc<Budget>, Memory) {
        let budget = Arc::new(Budget::new(limit));
        let memory = Memory::from(Arc::clone(&budget));
        (budget, memory)
    }

    /// The invariants every operation must keep: the reservation is the capacity in
    /// bytes, segments but the last are full-sized, and the elements match `model`.
    fn check<T: Clone + PartialEq + fmt::Debug>(vec: &SegVec<T>, model: &[T]) {
        let allocated: u64 = vec
            .segments
            .iter()
            .map(|segment| SegVec::<T>::bytes_of(segment.capacity()))
            .sum();
        assert_eq!(vec.reserved_bytes(), allocated, "reserved = allocated");
        assert_eq!(allocated, SegVec::<T>::bytes_of(vec.capacity()));
        if let Some((_, full)) = vec.segments.split_last() {
            assert!(full.iter().all(|s| s.capacity() == vec.segment_len()));
        }
        assert!(vec.segments.iter().all(|s| s.capacity() > 0));
        assert_eq!(vec.len(), model.len());
        assert!(vec.iter().eq(model.iter()));
        let segments: Vec<&[T]> = vec.segments().collect();
        assert_eq!(segments.concat(), model);
    }

    /// A small deterministic generator, so that the test needs no dependency.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, bound: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from((self.0 >> 33) % widen(bound)).unwrap()
        }
    }

    /// Random operations against a `Vec` model, under a budget tight enough that some
    /// of them are refused: after every operation, refused or not, the invariants
    /// hold, and a refused operation changed nothing.
    #[test]
    fn random_operations_keep_the_books_exact() {
        let rounds = 200;
        for bits in [0, 2, 5] {
            for limit in [64, 300, u64::MAX] {
                let (budget, memory) = budget(limit);
                let mut rng = Lcg(u64::from(bits) * 1000 + limit % 1000);
                let mut vec = SegVec::<u32>::with_segment_bits(&memory, "test", bits);
                let mut model: Vec<u32> = Vec::new();
                for round in 0..rounds {
                    let before = model.clone();
                    let value = u32::try_from(round).unwrap();
                    let result = match rng.below(9) {
                        0 | 1 => vec.try_push(value).map(|()| model.push(value)),
                        2 => {
                            let values: Vec<u32> = (0..rng.below(12)).map(|_| value).collect();
                            vec.try_extend_from_slice(&values)
                                .map(|()| model.extend_from_slice(&values))
                        }
                        3 => vec.try_reserve(rng.below(20)),
                        4 => {
                            let len = rng.below(model.len() + 10);
                            vec.try_resize(len, value)
                                .map(|()| model.resize(len, value))
                        }
                        5 => {
                            let len = rng.below(model.len() + 1);
                            vec.truncate(len);
                            model.truncate(len);
                            Ok(())
                        }
                        6 => {
                            vec.shrink_to_fit();
                            Ok(())
                        }
                        7 => vec.try_clone().map(|copy| check(&copy, &model)),
                        _ => {
                            if let Some(i) = model.len().checked_sub(1).map(|n| rng.below(n + 1)) {
                                vec[i] = value;
                                model[i] = value;
                            }
                            Ok(())
                        }
                    };
                    if let Err(error) = result {
                        assert!(matches!(error, MemoryError::Budget(_)), "{error}");
                        assert_eq!(model, before, "a refused operation changes nothing");
                    }
                    check(&vec, &model);
                    assert_eq!(budget.used(), vec.reserved_bytes());
                    assert!(budget.peak() <= limit);
                }
                drop(vec);
                assert_eq!(budget.used(), 0);
            }
        }
    }

    /// The acceptance case of issue #31: state larger than the budget is refused with
    /// the full size, before any segment is allocated. Here the size is 1 TiB, which no
    /// allocator could give (and a 32-bit target cannot address): had anything been
    /// allocated first, the error would be `AllocationFailed`, or the test would abort.
    #[test]
    fn state_beyond_the_budget_is_refused_before_allocating() {
        let (budget, memory) = budget(1 << 20);
        let nodes: u64 = 1 << 38;
        let needed = SegVec::<f32>::bytes_for(nodes).unwrap();
        assert_eq!(needed, 1 << 40);
        // A planner asks in u64 first, on every target.
        assert!(needed > memory.available());

        if let Ok(len) = usize::try_from(nodes) {
            let refused =
                SegVec::try_with_len(&memory, "pagerank ranks", len, 0.0_f32).unwrap_err();
            assert_eq!(
                refused,
                MemoryError::Budget(InsufficientBudget::new("pagerank ranks", 1 << 40, 1 << 20))
            );
        }
        // Just over the budget: refused too, with the exact shortfall.
        let refused = SegVec::try_with_len(&memory, "ranks", (1 << 18) + 1, 0_u32).unwrap_err();
        assert_eq!(
            refused,
            MemoryError::Budget(InsufficientBudget::new("ranks", (1 << 20) + 4, 1 << 20))
        );
        assert_eq!((budget.used(), budget.peak()), (0, 0));

        // Exactly the budget fits.
        let ranks = SegVec::try_with_len(&memory, "ranks", 1 << 18, 0_u32).unwrap();
        assert_eq!(budget.used(), 1 << 20);
        drop(ranks);
    }

    /// Reallocating the last segment counts the old and the new buffer together, and
    /// is refused when both do not fit, though the final size would.
    #[test]
    fn reallocation_reserves_both_buffers() {
        let (budget, memory) = budget(100);
        let mut vec = SegVec::<u8>::with_segment_bits(&memory, "bytes", 6);
        vec.try_reserve(40).unwrap();
        // 40 → 64 would end at 64 bytes, but needs 40 + 64 = 104 while it moves.
        assert!(matches!(vec.try_reserve(64), Err(MemoryError::Budget(_))));
        assert_eq!(vec.capacity(), 40);
        assert_eq!(budget.peak(), 40);
        vec.try_reserve(60).unwrap();
        assert_eq!(budget.peak(), 100);
        assert_eq!(budget.used(), 60);
    }

    #[test]
    fn growth_past_a_segment_adds_one_segment_at_a_time() {
        let memory = Memory::unlimited();
        let mut vec = SegVec::<u64>::with_segment_bits(&memory, "ids", 4);
        let mut capacities = Vec::new();
        for value in 0..16 {
            vec.try_push(value).unwrap();
            capacities.push(vec.capacity());
        }
        capacities.dedup();
        assert_eq!(capacities, [4, 8, 16], "doubling within the first segment");
        vec.try_push(16).unwrap();
        assert_eq!(vec.capacity(), 32, "not twice the memory");
        for value in 17..33 {
            vec.try_push(value).unwrap();
        }
        assert_eq!(vec.capacity(), 48);
        assert_eq!(vec.segments().len(), 3);
        assert_eq!(vec.as_slice(), None);
        assert_eq!(vec[20], 20);
        assert_eq!(vec.get(33), None);
    }

    #[test]
    fn range_slices_split_at_segment_ends() {
        let memory = Memory::unlimited();
        let mut vec = SegVec::<u32>::with_segment_bits(&memory, "targets", 2);
        let values: Vec<u32> = (0..11).collect();
        vec.try_extend_from_slice(&values).unwrap();
        for start in 0..=11 {
            for end in start..=11 {
                let parts: Vec<&[u32]> = vec.range_slices(start..end).collect();
                assert_eq!(parts.concat(), &values[start..end]);
                assert!(parts.iter().all(|p| !p.is_empty()));
                assert_eq!(
                    parts.len(),
                    (start..end).filter(|i| i % 4 == 0 || *i == start).count()
                );
            }
        }
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn range_slices_check_the_end() {
        let memory = Memory::unlimited();
        let vec = SegVec::try_with_len(&memory, "x", 3, 0_u8).unwrap();
        let _ = vec.range_slices(0..4).count();
    }

    /// Chunks are placed as on a flat slice, across segments, at any thread count.
    #[test]
    fn parallel_chunks_cover_the_vector_in_flat_positions() {
        let memory = Memory::unlimited();
        let mut vec = SegVec::try_with_len(&memory, "x", 37, 0_usize).unwrap();
        let mut flat = vec![0_usize; 37];
        let mark = |start: usize, chunk: &mut [usize]| {
            for (offset, value) in chunk.iter_mut().enumerate() {
                *value = start * 1000 + offset;
            }
        };
        let grain = NonZeroUsize::new(4).unwrap();
        par::for_each_chunk_mut(&mut flat, grain, mark);

        let mut small = SegVec::<usize>::with_segment_bits(&memory, "x", 3);
        small.try_resize(37, 0).unwrap();
        for target in [&mut vec, &mut small] {
            target.for_each_chunk_mut(grain, mark);
            assert!(target.iter().eq(flat.iter()));
        }
    }

    #[test]
    #[should_panic(expected = "does not divide")]
    fn a_grain_that_straddles_segments_is_rejected_on_any_length() {
        let memory = Memory::unlimited();
        let mut vec = SegVec::<u8>::with_segment_bits(&memory, "x", 3);
        vec.for_each_chunk_mut(NonZeroUsize::new(3).unwrap(), |_, _| {});
    }

    #[test]
    fn default_segments_stay_within_the_allocation_limit() {
        let memory = Memory::unlimited();
        assert_eq!(SegVec::<u8>::new(&memory, "x").segment_len(), 1 << 30);
        assert_eq!(SegVec::<f32>::new(&memory, "x").segment_len(), 1 << 28);
        assert_eq!(SegVec::<[u8; 3]>::new(&memory, "x").segment_len(), 1 << 28);
        assert_eq!(SegVec::<u64>::bytes_for(u64::MAX), None);
    }

    #[test]
    #[should_panic(expected = "exceed MAX_ALLOCATION_BYTES")]
    fn segments_above_the_limit_are_rejected() {
        let _ = SegVec::<u32>::with_segment_bits(&Memory::unlimited(), "x", 29);
    }

    #[test]
    fn a_clone_is_reserved_and_dropping_releases_it() {
        let (budget, memory) = budget(1000);
        let mut vec = SegVec::<u16>::with_segment_bits(&memory, "x", 3);
        vec.try_extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
            .unwrap();
        let held = vec.reserved_bytes();
        let copy = vec.try_clone().unwrap();
        assert_eq!(copy.reserved_bytes(), 20);
        assert_eq!(budget.used(), held + 20);
        assert!(copy.iter().eq(vec.iter()));
        drop(copy);
        vec.shrink_to_fit();
        assert_eq!(budget.used(), 20);
        vec.fill(7);
        assert!(vec.iter().all(|&v| v == 7));
        vec.clear();
        assert!(vec.is_empty());
        assert_eq!(vec.as_slice(), Some(&[][..]));
    }
}
