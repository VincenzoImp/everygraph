// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The parallel facade: the one place where work is split across threads.
//!
//! Kernels never name a thread pool. They cut their work into chunks and hand each
//! chunk to one of the functions below. With the `parallel` feature the chunks run on
//! rayon's current thread pool; without it they run in order on the calling thread.
//! The second mode is what a browser build without threads gets (FINDINGS § G), and it
//! is why no other crate in the workspace depends on rayon directly.
//!
//! Two properties hold in both modes and at any thread count:
//!
//! - **Chunk boundaries depend only on the length and the grain** passed in, never on
//!   how many threads exist. Chunk `i` covers `i * grain .. min((i + 1) * grain, len)`.
//! - **Results come back in chunk order.** [`map_ranges`] returns one value per chunk,
//!   first chunk first. Folding that vector on the calling thread gives the same answer
//!   everywhere, including for floating-point sums (DESIGN D14).
//!
//! # Choosing the grain
//!
//! **A grain is a function of the data, never of the machine.** Derive it from the
//! input (a constant, the block size of the format, the number of nodes) and from
//! nothing else. A grain computed from a thread count — `len / threads` is the classic
//! — moves the chunk boundaries with the hardware, and with them every floating-point
//! result: the same graph would give different answers on a laptop and on a server.
//! This module deliberately has no function that returns the number of threads.
//!
//! The grain is a [`NonZeroUsize`]: a grain of zero has no meaning, and a constant such
//! as `NonZeroUsize::new(1 << 16).unwrap()` is checked at compile time.
//!
//! The functions take the same `Send`/`Sync` bounds whether or not `parallel` is
//! enabled, so code that compiles in one mode compiles in the other.
//!
//! # Parallel writes without `unsafe`
//!
//! The core forbids `unsafe`, and the crates above it should not need it to write an
//! array in parallel. Abdi et al. (SPAA 2024, Table 3) sort the writes of parallel
//! programs into patterns; each pattern the engine uses has one helper here:
//!
//! - **Block**: task `i` writes elements `i * grain ..`. Use [`for_each_chunk_mut`]
//!   (or [`SegVec::for_each_chunk_mut`](crate::SegVec::for_each_chunk_mut)). Free.
//! - **Ranged indirect**: task `i` writes the slice between two offsets, as when each
//!   node's list of a CSR is sorted or each community's coarse edges are written. Use
//!   [`for_each_range_mut`] (or
//!   [`SegVec::for_each_range_mut`](crate::SegVec::for_each_range_mut)). The cost is
//!   a check that the offsets do not decrease, which the paper measures as negligible.
//! - **Single indirect**: each item goes to a position computed at run time, as when
//!   edges are moved to their source's bucket or nodes to their community. Use
//!   [`scatter`]: a counting sort over fixed blocks, stable, so items of one bucket
//!   keep their input order. With many keys (one per node), scatter into a few
//!   buckets of key ranges, then finish each bucket with [`for_each_range_mut`] (a
//!   sort of each bucket, or a counting pass within it).
//!
//! The pattern to avoid is the lab's: positions taken with `fetch_add` on a shared
//! counter and written through a raw pointer. Besides the `unsafe`, **the order it
//! writes depends on the schedule**: which thread reaches a counter first decides
//! which item gets the lower position, so the output changes from run to run and with
//! the thread count. That order must never reach a result (DESIGN D14), so such code
//! has to sort afterwards anyway. [`scatter`] writes the same order on every run.

use std::num::NonZeroUsize;
use std::ops::Range;

use crate::memory::{Memory, MemoryError, MemoryKind};
use crate::widen;

/// Calls `f` once for each chunk of `0..len`, possibly in parallel.
///
/// Use it for work whose only output goes through `f`'s captured state (atomics, for
/// example). When each chunk produces a value, use [`map_ranges`]; when each chunk
/// writes its own part of a slice, use [`for_each_chunk_mut`].
pub fn for_each_range<F>(len: usize, grain: NonZeroUsize, f: F)
where
    F: Fn(Range<usize>) + Send + Sync,
{
    let chunks = chunk_count(len, grain);
    let call = |chunk| f(chunk_range(chunk, len, grain));
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        (0..chunks).into_par_iter().for_each(call);
    }
    #[cfg(not(feature = "parallel"))]
    {
        (0..chunks).for_each(call);
    }
}

/// Calls `f` once for each chunk of `0..len`, possibly in parallel, and returns the
/// results in chunk order.
///
/// The returned vector has one element per chunk, `len.div_ceil(grain)` in all, and is
/// allocated here, outside any [`MemoryAccountant`](crate::MemoryAccountant). Keep `T`
/// small and the grain large enough for that to be negligible next to the data; a
/// caller with a tight budget reserves `len.div_ceil(grain) * size_of::<T>()` bytes
/// itself before calling.
pub fn map_ranges<T, F>(len: usize, grain: NonZeroUsize, f: F) -> Vec<T>
where
    T: Send,
    F: Fn(Range<usize>) -> T + Send + Sync,
{
    let chunks = chunk_count(len, grain);
    let call = |chunk| f(chunk_range(chunk, len, grain));
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        (0..chunks).into_par_iter().map(call).collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        (0..chunks).map(call).collect()
    }
}

/// Splits `data` into chunks of `grain` elements (the last one may be shorter) and
/// calls `f(start, chunk)` for each, possibly in parallel, where `start` is the index
/// in `data` of the chunk's first element.
///
/// Each call owns its chunk exclusively, which is how parallel writes to disjoint parts
/// of an array are expressed without `unsafe`.
pub fn for_each_chunk_mut<T, F>(data: &mut [T], grain: NonZeroUsize, f: F)
where
    T: Send,
    F: Fn(usize, &mut [T]) + Send + Sync,
{
    let call = |(chunk, slice): (usize, &mut [T])| f(chunk * grain.get(), slice);
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        data.par_chunks_mut(grain.get()).enumerate().for_each(call);
    }
    #[cfg(not(feature = "parallel"))]
    {
        data.chunks_mut(grain.get()).enumerate().for_each(call);
    }
}

/// Calls `f(i, &mut data[offsets[i]..offsets[i + 1]])` for each `i` below
/// `offsets.len() - 1`, possibly in parallel: the ranged writes of a CSR, one call per
/// list.
///
/// Offsets may be `usize`, `u64` or `u32`. They must not decrease, and the last must
/// not exceed `data.len()`; the first need not be zero. Elements outside
/// `offsets[0]..offsets[last]` are not handed out. Each call owns its range
/// exclusively: the ranges are cut from `data` with `split_at_mut`.
///
/// Calls are grouped into tasks of `grain` consecutive ranges, task `t` covering ranges
/// `t * grain ..`, so the boundaries depend on the offsets and the grain only. `f`
/// receives no task, so the grouping cannot change what it computes; the grain sets the
/// task size.
///
/// # Panics
///
/// If an offset is smaller than the one before it, or the last exceeds `data.len()`.
/// The offsets are checked as the data is split, so calls for other ranges may already
/// have been made.
pub fn for_each_range_mut<T, O, F>(data: &mut [T], offsets: &[O], grain: NonZeroUsize, f: F)
where
    T: Send,
    O: Copy + Sync,
    usize: TryFrom<O>,
    F: Fn(usize, &mut [T]) + Send + Sync,
{
    let Some(last) = offsets.len().checked_sub(1) else {
        return;
    };
    let (begin, end) = (offset_at(offsets, 0), offset_at(offsets, last));
    assert!(
        begin <= end && end <= data.len(),
        "offsets {begin}..{end} do not lie within data of length {}",
        data.len()
    );
    ranges_mut(&mut data[begin..end], begin, offsets, 0, grain.get(), &f);
}

/// Offset `i` as a `usize`; one that does not fit is beyond any data, and is reported
/// as such by the bounds checks.
pub(crate) fn offset_at<O: Copy>(offsets: &[O], i: usize) -> usize
where
    usize: TryFrom<O>,
{
    usize::try_from(offsets[i]).unwrap_or(usize::MAX)
}

/// Hands out ranges `first ..` of `offsets` (global indices), where `data` holds exactly
/// the elements `offsets[0]..offsets[last]` and `start` is the index of its first one.
/// Splits at task boundaries (multiples of `grain`) and recurses, halves in parallel.
pub(crate) fn ranges_mut<T, O, F>(
    data: &mut [T],
    start: usize,
    offsets: &[O],
    first: usize,
    grain: usize,
    f: &F,
) where
    T: Send,
    O: Copy + Sync,
    usize: TryFrom<O>,
    F: Fn(usize, &mut [T]) + Sync,
{
    let ranges = offsets.len() - 1;
    let end = first + ranges;
    // The first task boundary after `first`; none inside means one task.
    let boundary = (first / grain + 1)
        .checked_mul(grain)
        .filter(|&boundary| boundary < end);
    let Some(boundary) = boundary else {
        let mut rest = data;
        let mut at = start;
        for k in 0..ranges {
            let next = offset_at(offsets, k + 1);
            let len = next
                .checked_sub(at)
                .filter(|&len| len <= rest.len())
                .unwrap_or_else(|| bad_offset(first + k + 1, next));
            let (range, tail) = std::mem::take(&mut rest).split_at_mut(len);
            f(first + k, range);
            rest = tail;
            at = next;
        }
        return;
    };
    // Split at the middle task boundary.
    let mid = boundary + (end - 1 - boundary) / grain / 2 * grain;
    let at = offset_at(offsets, mid - first);
    let split = at
        .checked_sub(start)
        .filter(|&split| split <= data.len())
        .unwrap_or_else(|| bad_offset(mid, at));
    let (left, right) = data.split_at_mut(split);
    let (low, high) = (&offsets[..=mid - first], &offsets[mid - first..]);
    join(
        || ranges_mut(left, start, low, first, grain, f),
        || ranges_mut(right, at, high, mid, grain, f),
    );
}

fn bad_offset(index: usize, value: usize) -> ! {
    panic!("offsets[{index}] = {value} is below the offset before it or beyond the data")
}

/// Runs `a` and `b`, in parallel with the `parallel` feature.
fn join<A, B>(a: A, b: B)
where
    A: FnOnce() + Send,
    B: FnOnce() + Send,
{
    #[cfg(feature = "parallel")]
    {
        rayon::join(a, b);
    }
    #[cfg(not(feature = "parallel"))]
    {
        a();
        b();
    }
}

/// One cell of [`scatter`]: where the items of one bucket from one block go.
struct Cell<'a, T> {
    /// Items of the bucket in the block (pass 1).
    count: usize,
    /// The part of the output not yet written, and the parts after it when the cell
    /// crosses the end of an output slice (a [`SegVec`](crate::SegVec) segment).
    free: &'a mut [T],
    more: Vec<&'a mut [T]>,
}

impl<T> Cell<'_, T> {
    /// Writes `value` at the cell's next position; `false` if the cell is full.
    fn put(&mut self, value: T) -> bool {
        loop {
            if let Some((slot, rest)) = std::mem::take(&mut self.free).split_first_mut() {
                *slot = value;
                self.free = rest;
                return true;
            }
            if self.more.is_empty() {
                return false;
            }
            self.free = self.more.remove(0);
        }
    }
}

/// Moves `len` items into `buckets` buckets, stably: a parallel counting sort over
/// fixed blocks, for single indirect writes. Returns the bucket offsets (`buckets + 1`
/// of them): the items of bucket `b` end up at `offsets[b]..offsets[b + 1]` of the
/// output, in increasing order of their index `i`.
///
/// `item(i)` gives the bucket and the value of item `i`, for `i` in `0..len`. It is
/// called twice per item (to count, then to write) and must return the same both
/// times. `out` is the output as consecutive slices holding exactly `len` elements in
/// all: `[slice]` for one slice, [`SegVec::segments_mut`](crate::SegVec::segments_mut)
/// for a segmented array.
///
/// The items are cut into blocks of `grain` (block `k` is `k * grain ..`, as everywhere
/// in this module). Pass 1 counts each block's items per bucket; the counts give every
/// (block, bucket) pair its own part of the output, in bucket order and, within a
/// bucket, in block order; pass 2 lets each block write its parts. Hence the output
/// is a function of the items alone, whatever the thread count or the schedule.
///
/// The scratch is a table of `len.div_ceil(grain) × buckets` cells of six words each
/// (48 bytes on a 64-bit target), reserved against `memory` for `purpose` before
/// anything is allocated or called. That is why this is for a few buckets: with many
/// keys (one per node), scatter into buckets of key ranges and finish each with
/// [`for_each_range_mut`]. The returned vector of `buckets + 1` offsets, and the rare
/// extra parts of a cell that crosses the end of a slice, are allocated outside the
/// budget.
///
/// # Errors
///
/// [`MemoryError::Budget`] if the table does not fit in the budget, before `item` is
/// called; [`MemoryError::AllocationFailed`] if the allocator refused.
///
/// # Panics
///
/// If `item` returns a bucket not below `buckets`, if the slices of `out` do not hold
/// exactly `len` elements, or if `item` does not return the same bucket in both passes.
pub fn scatter<'a, T, F>(
    memory: &Memory,
    purpose: &'static str,
    len: usize,
    buckets: NonZeroUsize,
    grain: NonZeroUsize,
    item: F,
    out: impl IntoIterator<Item = &'a mut [T]>,
) -> Result<Vec<usize>, MemoryError>
where
    T: Send + 'a,
    F: Fn(usize) -> (usize, T) + Send + Sync,
{
    let width = buckets.get();
    let failed = |bytes| MemoryError::AllocationFailed { purpose, bytes };
    let cells = chunk_count(len, grain)
        .checked_mul(width)
        .ok_or(failed(u64::MAX))?;
    let bytes = widen(cells)
        .checked_mul(widen(size_of::<Cell<'a, T>>()))
        .ok_or(failed(u64::MAX))?;
    let _reservation = memory.reserve(purpose, MemoryKind::Anonymous, bytes)?;
    let mut table: Vec<Cell<'a, T>> = Vec::new();
    table.try_reserve_exact(cells).map_err(|_| failed(bytes))?;
    table.resize_with(cells, || Cell {
        count: 0,
        free: &mut [],
        more: Vec::new(),
    });

    // Pass 1: each block counts its items per bucket, in its own row of the table.
    for_each_chunk_mut(&mut table, buckets, |row_start, row| {
        for i in chunk_range(row_start / width, len, grain) {
            let bucket = item(i).0;
            match row.get_mut(bucket) {
                Some(cell) => cell.count += 1,
                None => panic!("item {i} is in bucket {bucket}, not below {width}"),
            }
        }
    });

    // Every cell its part of the output: bucket by bucket, block by block.
    let mut slices = out.into_iter();
    let mut current: &'a mut [T] = &mut [];
    let mut offsets = Vec::with_capacity(width + 1);
    let mut total = 0;
    for bucket in 0..width {
        offsets.push(total);
        for cell in table.iter_mut().skip(bucket).step_by(width) {
            total += cell.count;
            let mut need = cell.count;
            while need > 0 {
                while current.is_empty() {
                    current = slices
                        .next()
                        .unwrap_or_else(|| panic!("the output holds fewer than {len} elements"));
                }
                let take = need.min(current.len());
                let (part, rest) = std::mem::take(&mut current).split_at_mut(take);
                current = rest;
                need -= take;
                // Parts are never empty, so an empty `free` means "first part".
                if cell.free.is_empty() {
                    cell.free = part;
                } else {
                    cell.more.push(part);
                }
            }
        }
    }
    offsets.push(total);
    assert!(
        current.is_empty() && slices.all(|slice| slice.is_empty()),
        "the output holds more than {len} elements"
    );

    // Pass 2: each block writes its items into its cells.
    for_each_chunk_mut(&mut table, buckets, |row_start, row| {
        let block = row_start / width;
        for i in chunk_range(block, len, grain) {
            let (bucket, value) = item(i);
            assert!(
                row.get_mut(bucket).is_some_and(|cell| cell.put(value)),
                "item {i} changed bucket between the passes"
            );
        }
        assert!(
            row.iter()
                .all(|cell| cell.free.is_empty() && cell.more.is_empty()),
            "an item of block {block} changed bucket between the passes"
        );
    });
    Ok(offsets)
}

fn chunk_count(len: usize, grain: NonZeroUsize) -> usize {
    len.div_ceil(grain.get())
}

/// Range of chunk `chunk`, which must be below `chunk_count(len, grain)`.
fn chunk_range(chunk: usize, len: usize, grain: NonZeroUsize) -> Range<usize> {
    // `chunk < ceil(len / grain)` gives `chunk * grain < len`: no overflow here, and
    // the subtraction below cannot wrap. Adding the clamped grain cannot exceed `len`.
    let start = chunk * grain.get();
    start..start + grain.get().min(len - start)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{Budget, InsufficientBudget, SegVec};

    fn grain(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    /// The boundaries the module documents, computed the slow way.
    fn expected_ranges(len: usize, grain: usize) -> Vec<Range<usize>> {
        (0..len)
            .step_by(grain)
            .map(|start| start..len.min(start.saturating_add(grain)))
            .collect()
    }

    /// Lengths and grains around every boundary case: empty input, grain larger than
    /// the input, exact multiples, one element over, and the largest grain there is.
    const CASES: &[(usize, usize)] = &[
        (0, 1),
        (0, 8),
        (1, 1),
        (1, 8),
        (8, 8),
        (9, 8),
        (64, 8),
        (65, 8),
        (1000, 1),
        (1000, 7),
        (1000, 999),
        (1000, 1000),
        (1000, 1001),
        (1000, usize::MAX),
    ];

    #[test]
    fn map_ranges_returns_every_chunk_in_order() {
        for &(len, g) in CASES {
            let ranges = map_ranges(len, grain(g), |range| range);
            assert_eq!(ranges, expected_ranges(len, g), "len {len}, grain {g}");
        }
    }

    #[test]
    fn for_each_range_visits_the_same_chunks_once_each() {
        for &(len, g) in CASES {
            let seen = Mutex::new(Vec::new());
            for_each_range(len, grain(g), |range| seen.lock().unwrap().push(range));
            let mut seen = seen.into_inner().unwrap();
            seen.sort_unstable_by_key(|range| range.start);
            assert_eq!(seen, expected_ranges(len, g), "len {len}, grain {g}");
        }
    }

    #[test]
    fn for_each_chunk_mut_hands_out_every_element_once_with_its_position() {
        for &(len, g) in CASES {
            let mut data = vec![0_usize; len];
            let calls = AtomicU64::new(0);
            for_each_chunk_mut(&mut data, grain(g), |start, chunk| {
                calls.fetch_add(1, Ordering::Relaxed);
                assert!(chunk.len() <= g && !chunk.is_empty());
                for (i, slot) in chunk.iter_mut().enumerate() {
                    *slot += start + i + 1;
                }
            });
            let expected: Vec<usize> = (1..=len).collect();
            assert_eq!(data, expected, "len {len}, grain {g}");
            assert_eq!(
                calls.into_inner(),
                u64::try_from(expected_ranges(len, g).len()).unwrap()
            );
        }
    }

    /// Terms spread over forty orders of magnitude, so that their sum depends on how the
    /// additions are grouped. Only exactly rounded operations are used to build them:
    /// `powi` has unspecified precision, and Miri perturbs it on purpose.
    fn terms() -> Vec<f64> {
        const SCALES: [f64; 5] = [1e-20, 1e-10, 1.0, 1e10, 1e20];
        (1..=5000_u32)
            .zip(SCALES.iter().cycle())
            .map(|(i, scale)| {
                let sign = if i % 3 == 0 { -1.0 } else { 1.0 };
                sign * scale / f64::from(i)
            })
            .collect()
    }

    /// A reduction written the way kernels will write it: one partial per chunk, folded
    /// in chunk order on the calling thread.
    fn chunked_sum(values: &[f64], g: usize) -> f64 {
        map_ranges(values.len(), grain(g), |range| {
            values[range].iter().sum::<f64>()
        })
        .into_iter()
        .sum()
    }

    /// The same reduction with no facade at all.
    fn reference_sum(values: &[f64], g: usize) -> f64 {
        values
            .chunks(g)
            .map(|chunk| chunk.iter().sum::<f64>())
            .sum()
    }

    #[test]
    fn float_partials_come_back_in_chunk_order_bit_for_bit() {
        let values = terms();
        for g in [1, 3, 64, 1000, 5000, 9999] {
            let partials = map_ranges(values.len(), grain(g), |range| {
                values[range].iter().sum::<f64>().to_bits()
            });
            let expected: Vec<u64> = values
                .chunks(g)
                .map(|chunk| chunk.iter().sum::<f64>().to_bits())
                .collect();
            assert_eq!(partials, expected, "grain {g}");
            assert_eq!(
                chunked_sum(&values, g).to_bits(),
                reference_sum(&values, g).to_bits(),
                "grain {g}"
            );
        }
    }

    /// Why the boundaries must not move with the thread count: the same numbers grouped
    /// differently give a different sum. `1e16 + 1.0` rounds back to `1e16`, so adding
    /// the ones one at a time loses them all, while adding them in pairs does not.
    #[test]
    fn grouping_changes_a_float_sum() {
        let values = [1e16, 1.0, 1.0, 1.0, 1.0];
        assert_eq!(chunked_sum(&values, 5).to_bits(), 1e16_f64.to_bits());
        assert_eq!(
            chunked_sum(&values, 2).to_bits(),
            (1e16_f64 + 4.0).to_bits()
        );
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn results_do_not_depend_on_the_thread_count() {
        let values = terms();
        let expected = reference_sum(&values, 64).to_bits();
        for threads in [1, 2, 3, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            pool.install(|| {
                assert_eq!(rayon::current_num_threads(), threads);
                assert_eq!(chunked_sum(&values, 64).to_bits(), expected);

                let mut doubled = values.clone();
                for_each_chunk_mut(&mut doubled, grain(64), |start, chunk| {
                    for (i, value) in chunk.iter_mut().enumerate() {
                        *value = 2.0 * values[start + i];
                    }
                });
                assert!(doubled.iter().zip(&values).all(|(d, v)| *d == 2.0 * v));
            });
        }
    }

    /// A small deterministic generator, so that the tests need no dependency.
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

    fn to_u64(value: usize) -> u64 {
        u64::try_from(value).unwrap()
    }

    /// Random non-decreasing offsets within data of `len` elements, with empty ranges,
    /// a first offset above zero and a last one below `len` among the cases.
    fn random_offsets(rng: &mut Lcg, len: usize) -> Vec<usize> {
        let begin = rng.below(len + 1);
        let end = begin + rng.below(len - begin + 1);
        let ranges = rng.below(24);
        let mut offsets: Vec<usize> = (0..ranges.saturating_sub(1))
            .map(|_| begin + rng.below(end - begin + 1))
            .collect();
        offsets.sort_unstable();
        if ranges > 0 {
            offsets.insert(0, begin);
            offsets.push(end);
        } else if rng.below(2) == 0 {
            offsets.push(begin);
        }
        offsets
    }

    /// What every ranged test does to a range: checks that it is the right part of the
    /// data (elements start as their position), then writes something that depends on
    /// the range's index and on the order inside it.
    fn mark(offsets: &[usize], i: usize, range: &mut [u64]) {
        let start = offsets[i];
        assert_eq!(range.len(), offsets[i + 1] - start, "range {i}");
        assert!(range.iter().zip(start..).all(|(&x, p)| x == to_u64(p)));
        range.reverse();
        for x in range {
            *x += to_u64(i + 1) << 32;
        }
    }

    /// The result of `mark` on every range, computed one range at a time.
    fn marked(len: usize, offsets: &[usize]) -> Vec<u64> {
        let mut data: Vec<u64> = (0..to_u64(len)).collect();
        for i in 0..offsets.len().saturating_sub(1) {
            mark(offsets, i, &mut data[offsets[i]..offsets[i + 1]]);
        }
        data
    }

    /// Runs `mark` through the facade on a flat slice and on segmented copies, checking
    /// that each range is handed out exactly once, and returns the flat result.
    fn ranged(len: usize, offsets: &[usize], g: usize, bits: &[u32]) -> Vec<u64> {
        let ranges = offsets.len().saturating_sub(1);
        let calls: Vec<AtomicU64> = (0..ranges).map(|_| AtomicU64::new(0)).collect();
        let f = |i: usize, range: &mut [u64]| {
            calls[i].fetch_add(1, Ordering::Relaxed);
            mark(offsets, i, range);
        };
        let once = || calls.iter().all(|c| c.swap(0, Ordering::Relaxed) == 1);
        let mut flat: Vec<u64> = (0..to_u64(len)).collect();
        for_each_range_mut(&mut flat, offsets, grain(g), f);
        assert!(once());
        let memory = Memory::unlimited();
        for &b in bits {
            let mut vec = SegVec::<u64>::with_segment_bits(&memory, "ranges", b);
            vec.try_extend_from_slice(&(0..to_u64(len)).collect::<Vec<_>>())
                .unwrap();
            vec.for_each_range_mut(offsets, grain(g), f).unwrap();
            assert!(once(), "segments of 2^{b}");
            assert!(vec.iter().eq(flat.iter()), "segments of 2^{b}");
        }
        flat
    }

    const GRAINS: [usize; 6] = [1, 2, 3, 7, 64, usize::MAX];

    #[test]
    fn ranged_writes_hand_out_each_range_once_flat_and_segmented() {
        let mut rng = Lcg(28);
        for case in 0..30 {
            let len = rng.below(60);
            let offsets = random_offsets(&mut rng, len);
            let g = GRAINS[case % GRAINS.len()];
            let result = ranged(len, &offsets, g, &[0, 2, 27]);
            assert_eq!(result, marked(len, &offsets), "case {case}");
        }
    }

    #[test]
    fn ranged_writes_take_offsets_of_any_width() {
        let offsets = [1_usize, 1, 4, 9, 9];
        let expected = marked(10, &offsets);
        let narrow: Vec<u32> = offsets.iter().map(|&o| u32::try_from(o).unwrap()).collect();
        let wide: Vec<u64> = offsets.iter().map(|&o| to_u64(o)).collect();
        let mut a: Vec<u64> = (0..10).collect();
        let mut b = a.clone();
        for_each_range_mut(&mut a, &narrow, grain(2), |i, r| mark(&offsets, i, r));
        for_each_range_mut(&mut b, &wide, grain(2), |i, r| mark(&offsets, i, r));
        assert_eq!((a, b), (expected.clone(), expected));
    }

    #[test]
    #[should_panic(expected = "below the offset before it")]
    fn decreasing_offsets_are_rejected() {
        let mut data = [0_u8; 10];
        for_each_range_mut(&mut data, &[0_usize, 6, 3, 8], grain(64), |_, _| {});
    }

    #[test]
    #[should_panic(expected = "below the offset before it")]
    fn decreasing_offsets_are_rejected_at_a_task_boundary() {
        let mut data = [0_u8; 10];
        for_each_range_mut(&mut data, &[0_usize, 2, 9, 4, 5, 8], grain(2), |_, _| {});
    }

    #[test]
    #[should_panic(expected = "do not lie within data")]
    fn offsets_beyond_the_data_are_rejected() {
        let mut data = [0_u8; 10];
        for_each_range_mut(&mut data, &[2_u64, 11], grain(1), |_, _| {});
    }

    #[test]
    #[should_panic(expected = "offsets decrease")]
    fn segmented_ranges_check_the_offsets_before_calling() {
        let memory = Memory::unlimited();
        let mut vec = SegVec::try_with_len(&memory, "x", 10, 0_u8).unwrap();
        let _ = vec.for_each_range_mut(&[0_usize, 6, 3, 8], grain(1), |_, _| {
            panic!("called before the offsets were checked")
        });
    }

    /// A range that crosses a segment end is copied into scratch reserved first: when
    /// the budget has no room for it, nothing is called.
    #[test]
    fn crossing_ranges_reserve_their_scratch_first() {
        // Range 1 is 6..11: five elements across the end of the first segment.
        let offsets = [0_usize, 6, 11, 16];
        for room in [39, 40] {
            let budget = Arc::new(Budget::new(16 * 8 + room));
            let memory = Memory::from(Arc::clone(&budget));
            let mut vec = SegVec::<u64>::with_segment_bits(&memory, "lists", 3);
            vec.try_resize(16, 0).unwrap();
            let result = vec.for_each_range_mut(&offsets, grain(1), |i, r| {
                assert!(room == 40, "called without room for the scratch");
                r.fill(to_u64(i));
            });
            if room == 39 {
                let refused = InsufficientBudget::new("lists", 40, 39);
                assert_eq!(result, Err(MemoryError::Budget(refused)));
                assert!(vec.iter().all(|&x| x == 0));
            } else {
                assert_eq!(result, Ok(()));
                let expected = [0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2];
                assert!(vec.iter().eq(expected.iter()));
                assert_eq!(budget.peak(), 16 * 8 + 40);
            }
        }
    }

    /// Items with a bucket and a value that records where they came from; one case in
    /// three puts most items in bucket 0.
    fn items(seed: u64, len: usize, buckets: usize) -> Vec<(usize, u64)> {
        let mut rng = Lcg(seed);
        let skewed = rng.below(3) == 0;
        (0..len)
            .map(|i| {
                let bucket = if skewed && rng.below(4) > 0 {
                    0
                } else {
                    rng.below(buckets)
                };
                (bucket, to_u64(i) * 3 + 7)
            })
            .collect()
    }

    /// The stable counting sort, one bucket at a time.
    fn scattered(items: &[(usize, u64)], buckets: usize) -> (Vec<u64>, Vec<usize>) {
        let mut out = Vec::new();
        let mut offsets = vec![0];
        for b in 0..buckets {
            out.extend(items.iter().filter(|item| item.0 == b).map(|item| item.1));
            offsets.push(out.len());
        }
        (out, offsets)
    }

    /// `scatter` into a flat output and into segmented ones; returns the flat output.
    fn scattered_by_facade(items: &[(usize, u64)], b: usize, g: usize) -> Vec<u64> {
        let memory = Memory::unlimited();
        let (len, item) = (items.len(), |i: usize| items[i]);
        let expected = scattered(items, b).1;
        let mut out = vec![0_u64; len];
        let offsets = scatter(&memory, "s", len, grain(b), grain(g), item, [&mut out[..]]);
        assert_eq!(offsets.unwrap(), expected);
        for bits in [0, 3] {
            let mut vec = SegVec::<u64>::with_segment_bits(&memory, "out", bits);
            vec.try_resize(len, 0).unwrap();
            let offsets = scatter(
                &memory,
                "s",
                len,
                grain(b),
                grain(g),
                item,
                vec.segments_mut(),
            );
            assert_eq!(offsets.unwrap(), expected);
            assert!(vec.iter().eq(out.iter()), "segments of 2^{bits}");
        }
        out
    }

    #[test]
    fn scatter_is_a_stable_counting_sort_flat_and_segmented() {
        let mut rng = Lcg(189);
        for case in 0..30 {
            let (len, buckets) = (rng.below(80), 1 + rng.below(9));
            let g = GRAINS[case % GRAINS.len()];
            let items = items(to_u64(case), len, buckets);
            let out = scattered_by_facade(&items, buckets, g);
            assert_eq!(out, scattered(&items, buckets).0, "case {case}");
        }
    }

    #[test]
    fn scatter_reserves_its_table_before_calling_item() {
        let budget = Arc::new(Budget::new(100));
        let memory = Memory::from(Arc::clone(&budget));
        let mut out = [0_u8; 10];
        let item = |_| -> (usize, u8) { panic!("called before the table was reserved") };
        let refused = scatter(
            &memory,
            "parts",
            10,
            grain(3),
            grain(2),
            item,
            [&mut out[..]],
        );
        let needed = widen(5 * 3 * size_of::<Cell<'_, u8>>());
        let expected = InsufficientBudget::new("parts", needed, 100);
        assert_eq!(refused, Err(MemoryError::Budget(expected)));
        assert_eq!((budget.used(), budget.peak()), (0, 0));
    }

    fn scatter_bytes(
        len: usize,
        buckets: usize,
        out: usize,
        item: impl Fn(usize) -> (usize, u8) + Send + Sync,
    ) {
        let mut out = vec![0_u8; out];
        let memory = Memory::unlimited();
        let _ = scatter(
            &memory,
            "x",
            len,
            grain(buckets),
            grain(2),
            item,
            [&mut out[..]],
        );
    }

    #[test]
    #[should_panic(expected = "not below 3")]
    fn scatter_rejects_a_bucket_out_of_range() {
        scatter_bytes(4, 3, 4, |i| (i, 0));
    }

    #[test]
    #[should_panic(expected = "fewer than 5 elements")]
    fn scatter_rejects_a_short_output() {
        scatter_bytes(5, 1, 4, |_| (0, 0));
    }

    #[test]
    #[should_panic(expected = "more than 3 elements")]
    fn scatter_rejects_a_long_output() {
        scatter_bytes(3, 1, 4, |_| (0, 0));
    }

    #[test]
    #[should_panic(expected = "changed bucket")]
    fn scatter_rejects_an_item_that_changes_bucket() {
        let calls = AtomicU64::new(0);
        scatter_bytes(4, 2, 4, |i| {
            let second_pass = calls.fetch_add(1, Ordering::Relaxed) >= 4;
            (usize::from(i == 0 && second_pass), 0)
        });
    }

    /// The acceptance property of issue #189: each write helper gives the same bytes at
    /// 1, 3 and 8 threads, flat and segmented, equal to the sequential model.
    #[cfg(feature = "parallel")]
    #[test]
    fn writes_do_not_depend_on_the_thread_count() {
        let mut rng = Lcg(3);
        let len = 200;
        let mut offsets: Vec<usize> = (0..40).map(|_| rng.below(len + 1)).collect();
        offsets.sort_unstable();
        let items = items(8, len, 7);
        let mut results = Vec::new();
        for threads in [1, 3, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            results.push(pool.install(|| {
                assert_eq!(rayon::current_num_threads(), threads);
                let ranges = ranged(len, &offsets, 4, &[6]);
                (ranges, scattered_by_facade(&items, 7, 16))
            }));
        }
        assert_eq!(results[0], (marked(len, &offsets), scattered(&items, 7).0));
        assert!(results.iter().all(|result| *result == results[0]));
    }
}
