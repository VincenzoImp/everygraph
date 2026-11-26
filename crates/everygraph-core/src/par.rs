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
//! # Choosing the number of threads
//!
//! Work runs on whatever pool is current: rayon's global pool, or the one that
//! [`with_threads`] installs for the duration of a call. The thread count is the
//! caller's (the planner's) choice, made once outside the kernel; tests use
//! [`with_threads`] to run the same kernel at several thread counts in one process.
//!
//! # Parallel writes without `unsafe`
//!
//! The core forbids `unsafe`, and the crates above it should not need it to write an
//! array in parallel. Abdi et al. (SPAA 2024, Table 3) sort the writes of parallel
//! programs into patterns; each pattern the engine uses has one helper here:
//!
//! - **Block**: task `i` writes elements `i * grain ..`. Use [`for_each_chunk_mut`]
//!   (or [`SegVec::for_each_chunk_mut`](crate::SegVec::for_each_chunk_mut)). Free.
//! - **Ranged indirect**: task `i` writes the rows between two offsets, as when each
//!   node's list of a CSR is sorted or each community's coarse edges are written. Use
//!   [`for_each_range_mut`], or
//!   [`SegColumns::for_each_range_mut`](crate::SegColumns::for_each_range_mut) for
//!   segmented arrays. The cost is a check that the offsets do not decrease, which the
//!   paper measures as negligible.
//! - **Single indirect**: each item goes to a position computed at run time, as when
//!   edges are moved to their source's bucket or nodes to their community. Use
//!   [`scatter`]: a counting sort over the caller's blocks, stable, so the items of a
//!   bucket keep their input order. With many keys (one per node), scatter into a few
//!   buckets of key ranges, then finish each bucket with a ranged write.
//!
//! The ranged helpers and `scatter` write [`Columns`]: one slice, or several of equal
//! length (targets and their weights), cut at the same rows.
//!
//! The pattern to avoid is the lab's: positions taken with `fetch_add` on a shared
//! counter and written through a raw pointer. Besides the `unsafe`, **the order it
//! writes depends on the schedule**: which thread reaches a counter first decides
//! which item gets the lower position, so the output changes from run to run and with
//! the thread count. That order must never reach a result (DESIGN D14), so such code
//! has to sort afterwards anyway. [`scatter`] writes the same order on every run.

use std::error::Error;
use std::fmt;
use std::num::NonZeroUsize;
use std::ops::Range;

use crate::memory::{Memory, MemoryError, MemoryKind};
use crate::{MAX_ALLOCATION_BYTES, widen};

/// Runs `f` with the functions of this module spread over `threads` threads: a pool of
/// that size is made for the call and dropped after it. Without the `parallel` feature
/// `f` runs on the calling thread, as everything else here does.
///
/// Results do not depend on `threads` (see the module documentation); only the time
/// does.
///
/// # Errors
///
/// [`ThreadsError`] if the pool could not be made (the system refused a thread).
pub fn with_threads<R, F>(threads: NonZeroUsize, f: F) -> Result<R, ThreadsError>
where
    R: Send,
    F: FnOnce() -> R + Send,
{
    #[cfg(feature = "parallel")]
    {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads.get())
            .build()
            .map_err(|e| ThreadsError(e.to_string()))?;
        Ok(pool.install(f))
    }
    #[cfg(not(feature = "parallel"))]
    {
        let _ = threads;
        Ok(f())
    }
}

/// Why [`with_threads`] could not make its pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadsError(String);

impl fmt::Display for ThreadsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not start a thread pool: {}", self.0)
    }
}

impl Error for ThreadsError {}

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

/// The vector `[f(0), f(1), …, f(len - 1)]`, its elements computed in parallel in chunks
/// of `grain`, each chunk writing its own part of the vector.
///
/// For per-node state that `SegVec` cannot hold (atomics): a sequential fill of a large
/// vector is a single-threaded pass over memory that a parallel kernel then waits for.
/// The vector is allocated here, outside any [`MemoryAccountant`](crate::MemoryAccountant):
/// the caller reserves `len * size_of::<T>()` bytes first and keeps the reservation next
/// to it (CONTRIBUTING.md, "The allocation rule").
pub fn collect<T, F>(len: usize, grain: NonZeroUsize, f: F) -> Vec<T>
where
    T: Send,
    F: Fn(usize) -> T + Send + Sync,
{
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        (0..len)
            .into_par_iter()
            .with_min_len(grain.get())
            .map(f)
            .collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        let _ = grain;
        (0..len).map(f).collect()
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

/// Mutable columns of equal length that are cut at the same row: one slice, or a pair
/// of [`Columns`] (nest pairs for more). The write helpers below take them, so that a
/// list's targets and its property columns are written in one call.
pub trait Columns: Default + Send + Sized {
    /// One row: an element of every column.
    type Item;

    /// Number of rows.
    ///
    /// # Panics
    ///
    /// For a pair, if the columns have different lengths.
    fn rows(&self) -> usize;

    /// Rows `..mid` and rows `mid..`.
    ///
    /// # Panics
    ///
    /// If `mid` is larger than the number of rows.
    fn split_rows(self, mid: usize) -> (Self, Self);

    /// Writes `item` into the first row and removes that row from `self`.
    ///
    /// # Panics
    ///
    /// If there are no rows.
    fn put_first(&mut self, item: Self::Item);
}

impl<T: Send> Columns for &mut [T] {
    type Item = T;

    fn rows(&self) -> usize {
        self.len()
    }

    fn split_rows(self, mid: usize) -> (Self, Self) {
        self.split_at_mut(mid)
    }

    fn put_first(&mut self, item: T) {
        let Some((first, rest)) = std::mem::take(self).split_first_mut() else {
            panic!("no row to write into");
        };
        *first = item;
        *self = rest;
    }
}

impl<A: Columns, B: Columns> Columns for (A, B) {
    type Item = (A::Item, B::Item);

    fn rows(&self) -> usize {
        let rows = self.0.rows();
        assert_eq!(rows, self.1.rows(), "columns of different lengths");
        rows
    }

    fn split_rows(self, mid: usize) -> (Self, Self) {
        let ((a, b), (c, d)) = (self.0.split_rows(mid), self.1.split_rows(mid));
        ((a, c), (b, d))
    }

    fn put_first(&mut self, item: Self::Item) {
        self.0.put_first(item.0);
        self.1.put_first(item.1);
    }
}

/// Calls `f(i, rows offsets[i]..offsets[i + 1] of data)` for each `i` below
/// `offsets.len() - 1`, possibly in parallel: the ranged writes of a CSR, one call per
/// list. `data` is a slice (`&mut v[..]`) or a pair of slices ([`Columns`]).
///
/// Offsets may be `usize`, `u64` or `u32`. They must not decrease, and the last must
/// not be larger than the length of `data`. The first need not be zero. Rows outside
/// `offsets[0]..offsets[last]` are not handed out. Each call owns its range: the ranges
/// are cut from `data` with `split_at_mut`.
///
/// Calls are grouped into tasks of `grain` consecutive ranges. Task `t` covers ranges
/// `t * grain ..`. `f` does not see the task, so the grouping cannot change its result;
/// the grain only sets the task size.
///
/// # Panics
///
/// If an offset is smaller than the one before it, or the last is larger than the
/// length of `data`. The offsets are checked while the data is cut, so calls for other
/// ranges may already have run.
pub fn for_each_range_mut<D, O, F>(data: D, offsets: &[O], grain: NonZeroUsize, f: F)
where
    D: Columns,
    O: Copy + Sync,
    usize: TryFrom<O>,
    F: Fn(usize, D) + Send + Sync,
{
    let Some(last) = offsets.len().checked_sub(1) else {
        return;
    };
    let (begin, end, len) = (offset_at(offsets, 0), offset_at(offsets, last), data.rows());
    assert!(
        begin <= end && end <= len,
        "offsets {begin}..{end} do not lie within data of length {len}"
    );
    let data = data.split_rows(end).0.split_rows(begin).1;
    ranges_mut(data, begin, offsets, 0, grain.get(), &f);
}

/// Offset `i` as a `usize`. An offset that does not fit is beyond any data, and the
/// bounds checks report it as such.
pub(crate) fn offset_at<O: Copy>(offsets: &[O], i: usize) -> usize
where
    usize: TryFrom<O>,
{
    usize::try_from(offsets[i]).unwrap_or(usize::MAX)
}

/// Hands out ranges `first ..` (global indices) of `offsets`. `data` holds exactly the
/// rows `offsets[0]..offsets[last]`, and `start` is the index of its first row. Cuts at
/// the middle task boundary (a multiple of `grain`) and runs the halves in parallel.
pub(crate) fn ranges_mut<D, O, F>(
    data: D,
    start: usize,
    offsets: &[O],
    first: usize,
    grain: usize,
    f: &F,
) where
    D: Columns,
    O: Copy + Sync,
    usize: TryFrom<O>,
    F: Fn(usize, D) + Sync,
{
    let ranges = offsets.len() - 1;
    let end = first + ranges;
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
                .filter(|&len| len <= rest.rows())
                .unwrap_or_else(|| bad_offset(first + k + 1, next));
            let (range, tail) = std::mem::take(&mut rest).split_rows(len);
            f(first + k, range);
            rest = tail;
            at = next;
        }
        return;
    };
    let mid = boundary + (end - 1 - boundary) / grain / 2 * grain;
    let at = offset_at(offsets, mid - first);
    let split = at
        .checked_sub(start)
        .filter(|&split| split <= data.rows())
        .unwrap_or_else(|| bad_offset(mid, at));
    let (left, right) = data.split_rows(split);
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

/// Where the emitter of [`scatter`] sends the items of one block.
pub struct Sink<'s, D> {
    row: Row<'s, D>,
}

enum Row<'s, D> {
    /// First pass: items per bucket.
    Count(&'s mut [usize]),
    /// Second pass: the free rows of each bucket's cell, and the further parts of the
    /// cells of this block that cross the end of an output piece.
    Place(&'s mut [D], &'s mut Vec<(usize, D)>),
}

impl<D: Columns> Sink<'_, D> {
    /// Whether this is the first of the two calls of the block, which only counts: an
    /// emitter with side effects of its own (writing the items of other buckets
    /// elsewhere) does them once, in the second call.
    #[must_use]
    pub fn is_counting(&self) -> bool {
        matches!(self.row, Row::Count(_))
    }

    /// Sends `item` to `bucket`.
    ///
    /// # Panics
    ///
    /// If `bucket` is not below the number of buckets, or if the block sends more items
    /// to `bucket` in the second pass than in the first.
    pub fn push(&mut self, bucket: usize, item: D::Item) {
        fn more_than_counted(bucket: usize) -> ! {
            panic!("a block sent more items to bucket {bucket} than in the first pass")
        }
        match &mut self.row {
            Row::Count(row) => match row.get_mut(bucket) {
                Some(count) => *count += 1,
                None => panic!("bucket {bucket} is not below {}", row.len()),
            },
            Row::Place(row, more) => {
                let Some(cell) = row.get_mut(bucket) else {
                    more_than_counted(bucket)
                };
                while cell.rows() == 0 {
                    let Some(at) = more.iter().position(|part| part.0 == bucket) else {
                        more_than_counted(bucket)
                    };
                    *cell = more.remove(at).1;
                }
                cell.put_first(item);
            }
        }
    }
}

impl<D> fmt::Debug for Sink<'_, D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sink").finish_non_exhaustive()
    }
}

/// Moves items into `buckets` buckets, stably: a parallel counting sort over the
/// caller's blocks, for single indirect writes. Returns the bucket offsets
/// (`buckets + 1` of them): the items of bucket `b` are at rows
/// `offsets[b]..offsets[b + 1]` of the output, in block order, and within a block in
/// the order they were sent.
///
/// `emit(k, sink)` sends the items of block `k`, for `k` in `0..blocks`, with
/// [`Sink::push`]. It is called twice per block, to count and then to write, and must
/// send the same items in the same order both times. A block can be a chunk of an input
/// that is parsed again in each pass, so the items never need to be in memory. Reading
/// it can fail: an `Err` from `emit` ends the call with the error of the first block
/// that failed, after the pass in which it failed; the output is then unspecified.
///
/// `out` is the output as consecutive pieces of [`Columns`] that hold exactly as many
/// rows as items are sent: `[&mut v[..]]` for a slice, or
/// [`SegColumns::parts`](crate::SegColumns::parts) for segmented arrays. A row is one
/// item, so a pair of columns gets the targets and a property of each item in one call.
///
/// The output positions are a function of the counts only. Hence the result does not
/// depend on the thread count or the schedule (DESIGN D14).
///
/// **Waves.** To bound the output, call `scatter` once per wave of buckets: the emitter
/// sends only the items whose bucket is in the wave, numbered from the wave's first
/// bucket, and `out` holds only those items. Each wave reads the input twice.
///
/// **A CSR build** (#35, pass 2) can use it this way. Buckets are ranges of source
/// nodes. `out` is the wave's part of the final targets array (4 bytes per edge) and a
/// column of source ids local to the bucket (2 to 4 bytes per edge, for the edges of the
/// wave only), paired with [`zip_pieces`]. Then a ranged write over the buckets moves
/// each bucket's targets to their nodes' lists, with one bucket-sized buffer per thread. The total is 4 bytes
/// per edge, plus 2 to 4 bytes per edge of the largest wave, plus the table below. This
/// is arithmetic, not a measurement.
///
/// **Memory.** The table has `blocks × buckets` cells. Each cell is a count
/// (`size_of::<usize>()`) and a part of the output (`size_of::<D>()`: 16 bytes for one
/// slice on a 64-bit target, 8 bytes on a 32-bit one), so the same call can fit a budget
/// on a 32-bit target and not on a 64-bit one. Both arrays and the per-block lists are
/// reserved against `memory` for `purpose` before anything is allocated or `emit` is
/// called. Each must be one allocation of at most
/// [`MAX_ALLOCATION_BYTES`]: choose blocks and buckets
/// with `blocks × buckets × size_of::<D>()` at most 1 GiB. For example, 3.6 G edges in
/// blocks of 2^20 edges and 2,048 buckets make 7.0 M cells: 112 MB of parts on a 64-bit
/// target. The returned offsets, and the rare extra parts of a cell that crosses the end
/// of a piece, are allocated outside the budget.
///
/// # Errors
///
/// [`MemoryError::AllocationFailed`] if an array of the table would be larger than
/// `MAX_ALLOCATION_BYTES` or the allocator refused it, and [`MemoryError::Budget`] if
/// the table does not fit in the budget, in both cases before `emit` is called; the
/// first error of `emit`.
///
/// # Panics
///
/// If `emit` sends an item to a bucket not below `buckets`, if the pieces of `out` do not
/// hold exactly as many rows as items sent, or if a block that did not fail sends other
/// items in the second pass than in the first.
pub fn scatter<D, E, X>(
    memory: &Memory,
    purpose: &'static str,
    blocks: usize,
    buckets: NonZeroUsize,
    emit: E,
    out: impl IntoIterator<Item = D>,
) -> Result<Vec<usize>, X>
where
    D: Columns,
    E: Fn(usize, &mut Sink<'_, D>) -> Result<(), X> + Send + Sync,
    X: From<MemoryError> + Send,
{
    let width = buckets.get();
    let failed = |bytes| MemoryError::AllocationFailed { purpose, bytes };
    let cells = blocks.checked_mul(width).ok_or(failed(u64::MAX))?;
    let mut bytes = 0_u64;
    for (count, size) in [
        (cells, size_of::<usize>()),
        (cells, size_of::<D>()),
        (blocks, size_of::<Vec<(usize, D)>>()),
        (blocks, size_of::<(&mut [D], &mut Vec<(usize, D)>)>()),
    ] {
        let array = widen(count).saturating_mul(widen(size));
        if array > MAX_ALLOCATION_BYTES {
            return Err(failed(array).into());
        }
        bytes += array;
    }
    let _reservation = memory
        .reserve(purpose, MemoryKind::Anonymous, bytes)
        .map_err(MemoryError::from)?;
    let mut counts = filled(cells, || 0, failed)?;
    let mut parts: Vec<D> = filled(cells, D::default, failed)?;
    let mut more: Vec<Vec<(usize, D)>> = filled(blocks, Vec::new, failed)?;
    let mut jobs = Vec::new();
    jobs.try_reserve_exact(blocks)
        .map_err(|_| failed(widen(blocks)))?;

    let failed_blocks = std::sync::Mutex::new(Vec::new());
    let fail = |block: usize, error: X| {
        failed_blocks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((block, error));
    };
    let first_failure = |failed: std::sync::Mutex<Vec<(usize, X)>>| {
        let failed = failed
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        failed.into_iter().min_by_key(|(block, _)| *block)
    };
    for_each_chunk_mut(&mut counts, buckets, |start, row| {
        let block = start / width;
        let mut sink = Sink {
            row: Row::Count(row),
        };
        if let Err(error) = emit(block, &mut sink) {
            fail(block, error);
        }
    });
    if let Some((_, error)) = first_failure(failed_blocks) {
        return Err(error);
    }
    let failed_blocks = std::sync::Mutex::new(Vec::new());
    let fail = |block: usize, error: X| {
        failed_blocks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((block, error));
    };

    // Each cell gets its rows of the output: bucket by bucket, then block by block.
    let mut pieces = out.into_iter();
    let mut piece = D::default();
    let mut offsets = Vec::with_capacity(width + 1);
    let mut total = 0;
    for bucket in 0..width {
        offsets.push(total);
        for (block, more) in more.iter_mut().enumerate() {
            let cell = block * width + bucket;
            let mut need = counts[cell];
            total += need;
            while need > 0 {
                while piece.rows() == 0 {
                    piece = pieces.next().unwrap_or_else(|| {
                        panic!("the output holds fewer rows than the items sent")
                    });
                }
                let take = need.min(piece.rows());
                let (part, rest) = std::mem::take(&mut piece).split_rows(take);
                piece = rest;
                need -= take;
                if parts[cell].rows() == 0 {
                    parts[cell] = part;
                } else {
                    more.push((bucket, part));
                }
            }
        }
    }
    offsets.push(total);
    assert!(
        piece.rows() == 0 && pieces.all(|piece| piece.rows() == 0),
        "the output holds more rows than the items sent"
    );

    jobs.extend(parts.chunks_mut(width).zip(more.iter_mut()));
    for_each_chunk_mut(&mut jobs, NonZeroUsize::MIN, |block, job| {
        let (row, more) = &mut job[0];
        let mut sink = Sink {
            row: Row::Place(row, more),
        };
        if let Err(error) = emit(block, &mut sink) {
            fail(block, error);
            return;
        }
        assert!(
            row.iter().all(|cell| cell.rows() == 0) && more.is_empty(),
            "block {block} sent fewer items than in the first pass"
        );
    });
    match first_failure(failed_blocks) {
        Some((_, error)) => Err(error),
        None => Ok(offsets),
    }
}

/// Pairs two sequences of pieces that hold the same rows but are cut at different
/// places, such as a range of one [`SegVec`](crate::SegVec) and a whole other one: each
/// output piece ends where a piece of either input ends. For the `out` of [`scatter`].
///
/// # Panics
///
/// While iterating, if the two sequences hold different numbers of rows.
pub fn zip_pieces<A: Columns, B: Columns>(
    a: impl IntoIterator<Item = A>,
    b: impl IntoIterator<Item = B>,
) -> impl Iterator<Item = (A, B)> {
    let (mut a, mut b) = (a.into_iter(), b.into_iter());
    let (mut x, mut y) = (A::default(), B::default());
    std::iter::from_fn(move || {
        while x.rows() == 0 {
            let Some(next) = a.next() else {
                let done = y.rows() == 0 && b.all(|piece| piece.rows() == 0);
                assert!(done, "pieces of different total lengths");
                return None;
            };
            x = next;
        }
        while y.rows() == 0 {
            y = b.next().expect("pieces of different total lengths");
        }
        let take = x.rows().min(y.rows());
        let (p, rest) = std::mem::take(&mut x).split_rows(take);
        x = rest;
        let (q, rest) = std::mem::take(&mut y).split_rows(take);
        y = rest;
        Some((p, q))
    })
}

/// A vector of `len` values made by `value`, allocated fallibly. The caller has
/// reserved it.
fn filled<X>(
    len: usize,
    value: impl FnMut() -> X,
    failed: impl Fn(u64) -> MemoryError,
) -> Result<Vec<X>, MemoryError> {
    let mut vec = Vec::new();
    vec.try_reserve_exact(len)
        .map_err(|_| failed(widen(len).saturating_mul(widen(size_of::<X>()))))?;
    vec.resize_with(len, value);
    Ok(vec)
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
    use crate::{Budget, InsufficientBudget, SegColumns, SegVec};

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
    fn collect_puts_every_element_at_its_index() {
        for &(len, g) in CASES {
            let v = collect(len.min(5000), grain(g), |i| 3 * i + 1);
            assert!(v.iter().enumerate().all(|(i, &x)| x == 3 * i + 1));
            assert_eq!(v.len(), len.min(5000), "len {len}, grain {g}");
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

    /// The pool `with_threads` installs is the one the facade's functions run on.
    #[test]
    fn with_threads_runs_the_facade_on_a_pool_of_that_size() {
        let values = terms();
        let expected = reference_sum(&values, 64).to_bits();
        for threads in [1, 3, 8] {
            let (sum, seen) = with_threads(grain(threads), || {
                let workers = Mutex::new(std::collections::BTreeSet::new());
                for_each_range(1 << 12, grain(1), |_| {
                    workers.lock().unwrap().insert(rayon_index().unwrap_or(0));
                });
                (
                    chunked_sum(&values, 64).to_bits(),
                    workers.into_inner().unwrap(),
                )
            })
            .unwrap();
            assert_eq!(sum, expected, "threads {threads}");
            if cfg!(feature = "parallel") {
                assert!(seen.len() <= threads && seen.iter().all(|&w| w < threads));
            }
        }
    }

    /// The index of the current worker in the current pool, when there is one.
    fn rayon_index() -> Option<usize> {
        #[cfg(feature = "parallel")]
        {
            rayon::current_thread_index()
        }
        #[cfg(not(feature = "parallel"))]
        {
            None
        }
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

    fn to_u32(value: usize) -> u32 {
        u32::try_from(value).unwrap()
    }

    /// Random non-decreasing offsets within data of `len` rows, with empty ranges, a
    /// first offset above zero and a last one below `len` among the cases.
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
    /// data (rows start as their position), then writes something that depends on the
    /// range's index and on the order inside it.
    fn mark(offsets: &[usize], i: usize, range: &mut [u64]) {
        let start = offsets[i];
        assert_eq!(range.len(), offsets[i + 1] - start, "range {i}");
        assert!(range.iter().zip(start..).all(|(&x, p)| x == to_u64(p)));
        range.reverse();
        for x in range {
            *x += to_u64(i + 1) << 32;
        }
    }

    /// `mark` on the first column; the second, also positions, is reversed.
    fn mark_pair(offsets: &[usize], i: usize, (a, b): (&mut [u64], &mut [u32])) {
        assert!(b.iter().zip(offsets[i]..).all(|(&x, p)| x == to_u32(p)));
        b.reverse();
        mark(offsets, i, a);
    }

    /// The result of `mark_pair` on every range, computed one range at a time.
    fn marked(len: usize, offsets: &[usize]) -> (Vec<u64>, Vec<u32>) {
        let mut a: Vec<u64> = (0..to_u64(len)).collect();
        let mut b: Vec<u32> = (0..to_u32(len)).collect();
        for i in 0..offsets.len().saturating_sub(1) {
            let range = offsets[i]..offsets[i + 1];
            mark_pair(offsets, i, (&mut a[range.clone()], &mut b[range]));
        }
        (a, b)
    }

    /// Runs `mark` through the facade on one and two columns, flat and segmented,
    /// checking that each range is handed out exactly once; returns the flat result.
    fn ranged(len: usize, offsets: &[usize], g: usize, bits: &[u32]) -> (Vec<u64>, Vec<u32>) {
        let ranges = offsets.len().saturating_sub(1);
        let calls: Vec<AtomicU64> = (0..ranges).map(|_| AtomicU64::new(0)).collect();
        let call = |i: usize| calls[i].fetch_add(1, Ordering::Relaxed);
        let calls_were = |n| calls.iter().all(|c| c.swap(0, Ordering::Relaxed) == n);
        let once = || calls_were(1);
        let one = |i, a: &mut [u64]| {
            call(i);
            mark(offsets, i, a);
        };
        let two = |i, columns: (&mut [u64], &mut [u32])| {
            call(i);
            mark_pair(offsets, i, columns);
        };
        let positions: Vec<u64> = (0..to_u64(len)).collect();
        let mut a = positions.clone();
        for_each_range_mut(&mut a[..], offsets, grain(g), one);
        assert!(once());
        let mut pair = (positions.clone(), (0..to_u32(len)).collect::<Vec<_>>());
        for_each_range_mut((&mut pair.0[..], &mut pair.1[..]), offsets, grain(g), two);
        assert!(once() && pair.0 == a);
        // A range longer than a segment is refused before any call, data unchanged.
        let longest = offsets.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0);
        let memory = Memory::unlimited();
        for &b in bits {
            let fits = longest <= 1 << b;
            let (expected, times) = if fits { (&a, 1) } else { (&positions, 0) };
            let check = |result: Result<(), MemoryError>| {
                assert_eq!(result.is_ok(), fits, "segments of 2^{b}");
                assert!(calls_were(times), "segments of 2^{b}");
            };
            let mut first = SegVec::<u64>::with_segment_bits(&memory, "ranges", b);
            first.try_extend_from_slice(&positions).unwrap();
            check(first.for_each_range_mut(offsets, grain(g), one));
            assert!(first.iter().eq(expected.iter()), "segments of 2^{b}");
            first.fill(0);
            first.for_each_chunk_mut(grain(1), |p, x| x[0] = to_u64(p));
            let mut second = SegVec::<u32>::with_segment_bits(&memory, "ranges", b);
            second
                .try_extend_from_slice(&(0..to_u32(len)).collect::<Vec<_>>())
                .unwrap();
            check((&mut first, &mut second).for_each_range_mut(offsets, grain(g), two));
            assert!(first.iter().eq(expected.iter()), "segments of 2^{b}");
            let second_expected: Vec<u32> = if fits {
                pair.1.clone()
            } else {
                (0..to_u32(len)).collect()
            };
            assert!(
                second.iter().eq(second_expected.iter()),
                "segments of 2^{b}"
            );
        }
        pair
    }

    const GRAINS: [usize; 6] = [1, 2, 3, 7, 64, usize::MAX];

    #[test]
    fn ranged_writes_hand_out_each_range_once_flat_and_segmented() {
        let mut rng = Lcg(28);
        for case in 0..18 {
            let len = rng.below(48);
            let offsets = random_offsets(&mut rng, len);
            let g = GRAINS[case % GRAINS.len()];
            let result = ranged(len, &offsets, g, &[0, 2, 4, 27]);
            assert_eq!(result, marked(len, &offsets), "case {case}");
        }
    }

    #[test]
    fn ranged_writes_take_offsets_of_any_width() {
        let offsets = [1_usize, 1, 4, 9, 9];
        let expected = marked(10, &offsets).0;
        let narrow: Vec<u32> = offsets.iter().map(|&o| to_u32(o)).collect();
        let wide: Vec<u64> = offsets.iter().map(|&o| to_u64(o)).collect();
        let mut a: Vec<u64> = (0..10).collect();
        let mut b = a.clone();
        for_each_range_mut(&mut a[..], &narrow, grain(2), |i, r| mark(&offsets, i, r));
        for_each_range_mut(&mut b[..], &wide, grain(2), |i, r| mark(&offsets, i, r));
        assert_eq!((a, b), (expected.clone(), expected));
    }

    #[test]
    #[should_panic(expected = "below the offset before it")]
    fn decreasing_offsets_are_rejected() {
        let mut data = [0_u8; 10];
        for_each_range_mut(&mut data[..], &[0_usize, 6, 3, 8], grain(64), |_, _| {});
    }

    #[test]
    #[should_panic(expected = "below the offset before it")]
    fn decreasing_offsets_are_rejected_at_a_task_boundary() {
        let mut data = [0_u8; 10];
        let offsets = [0_usize, 2, 9, 4, 5, 8];
        for_each_range_mut(&mut data[..], &offsets, grain(2), |_, _| {});
    }

    #[test]
    #[should_panic(expected = "do not lie within data")]
    fn offsets_beyond_the_data_are_rejected() {
        let mut data = [0_u8; 10];
        for_each_range_mut(&mut data[..], &[2_u64, 11], grain(1), |_, _| {});
    }

    #[test]
    #[should_panic(expected = "columns of different lengths")]
    fn columns_of_different_lengths_are_rejected() {
        let (mut a, mut b) = ([0_u8; 4], [0_u8; 5]);
        for_each_range_mut((&mut a[..], &mut b[..]), &[0_usize, 4], grain(1), |_, _| {});
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
        // Range 1 is 6..11: five rows across the end of the first segment.
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

    /// A list longer than a segment can never be one slice: refused before any call.
    #[test]
    fn a_range_longer_than_a_segment_is_refused() {
        let memory = Memory::unlimited();
        let mut vec = SegVec::<u32>::with_segment_bits(&memory, "targets", 2);
        vec.try_resize(12, 0).unwrap();
        let result = vec.for_each_range_mut(&[0_usize, 1, 6, 12], grain(1), |_, _| {
            panic!("called although a range is longer than a segment")
        });
        let refused = MemoryError::AllocationFailed {
            purpose: "targets",
            bytes: 6 * 4,
        };
        assert_eq!(result, Err(refused));
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

    /// The stable counting sort, one bucket at a time: values, their item indices, and
    /// the bucket offsets.
    fn scattered(items: &[(usize, u64)], buckets: usize) -> (Vec<u64>, Vec<u32>, Vec<usize>) {
        let (mut values, mut indices, mut offsets) = (Vec::new(), Vec::new(), vec![0]);
        for b in 0..buckets {
            for (i, item) in items.iter().enumerate().filter(|(_, item)| item.0 == b) {
                values.push(item.1);
                indices.push(to_u32(i));
            }
            offsets.push(values.len());
        }
        (values, indices, offsets)
    }

    /// `scatter` with blocks of `g` items, sending only the buckets of `wave`, numbered
    /// from its start.
    fn scatter_wave<D: Columns>(
        items: &[(usize, u64)],
        buckets: Range<usize>,
        g: usize,
        value: impl Fn(usize, u64) -> D::Item + Send + Sync,
        out: impl IntoIterator<Item = D>,
    ) -> Vec<usize> {
        let blocks = items.len().div_ceil(g);
        let width = grain(buckets.len().max(1));
        let emit = |block: usize, sink: &mut Sink<'_, D>| {
            for (i, &(bucket, v)) in items.iter().enumerate().skip(block * g).take(g) {
                if buckets.contains(&bucket) {
                    sink.push(bucket - buckets.start, value(i, v));
                }
            }
            Ok::<_, MemoryError>(())
        };
        scatter(&Memory::unlimited(), "s", blocks, width, emit, out).unwrap()
    }

    /// `scatter` into one and two columns, flat and segmented, and of a wave into the
    /// middle of a segmented array; returns the flat output.
    fn scattered_by_facade(
        items: &[(usize, u64)],
        b: usize,
        g: usize,
        bits: &[u32],
    ) -> (Vec<u64>, Vec<u32>) {
        let (values, indices, offsets) = scattered(items, b);
        let len = items.len();
        let mut one = vec![0_u64; len];
        assert_eq!(
            scatter_wave(items, 0..b, g, |_, v| v, [&mut one[..]]),
            offsets
        );
        let mut two = (vec![0_u64; len], vec![0_u32; len]);
        let pair = (&mut two.0[..], &mut two.1[..]);
        let both = |i, v| (v, to_u32(i));
        assert_eq!(scatter_wave(items, 0..b, g, both, [pair]), offsets);
        assert_eq!((&one, &two.0, &two.1), (&values, &values, &indices));
        let memory = Memory::unlimited();
        for &bits in bits {
            let mut first = SegVec::<u64>::with_segment_bits(&memory, "out", bits);
            let mut second = SegVec::<u32>::with_segment_bits(&memory, "out", bits);
            first.try_resize(len, 0).unwrap();
            second.try_resize(len, 0).unwrap();
            let mut columns = (&mut first, &mut second);
            let parts = columns.parts(0..len);
            assert_eq!(scatter_wave(items, 0..b, g, both, parts), offsets);
            assert!(first.iter().eq(values.iter()), "segments of 2^{bits}");
            assert!(second.iter().eq(indices.iter()), "segments of 2^{bits}");
            // A wave of the middle buckets: into their rows of a cleared array, and a
            // column of the wave only, cut at other places.
            first.fill(0);
            let wave = b / 3..b - b / 3;
            let rows = offsets[wave.start]..offsets[wave.end];
            let mut column = &mut first;
            let mut local = SegVec::<u32>::with_segment_bits(&memory, "out", 1);
            local.try_resize(rows.len(), 0).unwrap();
            let mut local_column = &mut local;
            let parts = zip_pieces(
                column.parts(rows.clone()),
                local_column.parts(0..rows.len()),
            );
            let got = scatter_wave(items, wave.clone(), g, both, parts);
            let expected: Vec<usize> = offsets[wave.start..=wave.end]
                .iter()
                .map(|o| o - rows.start)
                .collect();
            assert_eq!(got, expected);
            let mut model = vec![0; len];
            model[rows.clone()].copy_from_slice(&values[rows.clone()]);
            assert!(first.iter().eq(model.iter()), "wave, segments of 2^{bits}");
            assert!(
                local.iter().eq(indices[rows].iter()),
                "wave, segments of 2^{bits}"
            );
        }
        (one, two.1)
    }

    #[test]
    fn scatter_is_a_stable_counting_sort_flat_and_segmented() {
        let mut rng = Lcg(189);
        for case in 0..18 {
            let (len, buckets) = (rng.below(48), 1 + rng.below(9));
            let g = GRAINS[case % GRAINS.len()];
            let items = items(to_u64(case), len, buckets);
            let (values, indices, _) = scattered(&items, buckets);
            let out = scattered_by_facade(&items, buckets, g, &[0, 3]);
            assert_eq!(out, (values, indices), "case {case}");
        }
    }

    fn no_items(_: usize, _: &mut Sink<'_, &mut [u8]>) -> Result<(), MemoryError> {
        panic!("called before the table was reserved");
    }

    #[test]
    fn scatter_reserves_its_table_before_calling_emit() {
        let budget = Arc::new(Budget::new(100));
        let memory = Memory::from(Arc::clone(&budget));
        let mut out = [0_u8; 10];
        let refused = scatter(&memory, "parts", 5, grain(3), no_items, [&mut out[..]]);
        let cell = size_of::<usize>() + size_of::<&mut [u8]>();
        let block = size_of::<Vec<(usize, &mut [u8])>>() + size_of::<(&mut [u8], &mut Vec<u8>)>();
        let needed = widen(5 * 3 * cell + 5 * block);
        let expected = InsufficientBudget::new("parts", needed, 100);
        assert_eq!(refused, Err(MemoryError::Budget(expected)));
        assert_eq!((budget.used(), budget.peak()), (0, 0));
    }

    /// The cell table must be one allocation of at most `MAX_ALLOCATION_BYTES`, also when
    /// the budget has room.
    #[test]
    fn scatter_refuses_a_table_above_one_allocation() {
        let mut out = [0_u8; 0];
        let memory = Memory::unlimited();
        let refused = scatter(
            &memory,
            "parts",
            1 << 15,
            grain(1 << 14),
            no_items,
            [&mut out[..]],
        );
        let bytes = widen(1 << 29) * widen(size_of::<usize>());
        assert_eq!(
            refused,
            Err(MemoryError::AllocationFailed {
                purpose: "parts",
                bytes
            })
        );
    }

    fn scatter_bytes(
        items: usize,
        buckets: usize,
        out: usize,
        bucket: impl Fn(usize) -> usize + Send + Sync,
    ) {
        let mut out = vec![0_u8; out];
        let memory = Memory::unlimited();
        let emit = |block: usize, sink: &mut Sink<'_, &mut [u8]>| {
            for i in block * 2..items.min(block * 2 + 2) {
                sink.push(bucket(i), 0);
            }
            Ok::<_, MemoryError>(())
        };
        let blocks = items.div_ceil(2);
        let _ = scatter(&memory, "x", blocks, grain(buckets), emit, [&mut out[..]]);
    }

    #[test]
    #[should_panic(expected = "pieces of different total lengths")]
    fn zip_pieces_rejects_pieces_of_different_lengths() {
        let (mut a, mut b) = ([0_u8; 5], [0_u8; 4]);
        let (x, y) = a.split_at_mut(2);
        let _ = zip_pieces([x, y], [&mut b[..]]).count();
    }

    #[test]
    #[should_panic(expected = "bucket 3 is not below 3")]
    fn scatter_rejects_a_bucket_out_of_range() {
        scatter_bytes(4, 3, 4, |i| i);
    }

    #[test]
    #[should_panic(expected = "fewer rows than the items sent")]
    fn scatter_rejects_a_short_output() {
        scatter_bytes(5, 1, 4, |_| 0);
    }

    #[test]
    #[should_panic(expected = "more rows than the items sent")]
    fn scatter_rejects_a_long_output() {
        scatter_bytes(3, 1, 4, |_| 0);
    }

    #[test]
    #[should_panic(expected = "more items to bucket 1")]
    fn scatter_rejects_a_block_that_sends_more_in_the_second_pass() {
        let calls = AtomicU64::new(0);
        scatter_bytes(2, 2, 2, |i| {
            let second_pass = calls.fetch_add(1, Ordering::Relaxed) >= 2;
            usize::from(i == 0 && second_pass)
        });
    }

    #[test]
    #[should_panic(expected = "sent fewer items than in the first pass")]
    fn scatter_rejects_a_block_that_sends_fewer_in_the_second_pass() {
        let calls = AtomicU64::new(0);
        let mut out = [0_u8; 2];
        let emit = |_: usize, sink: &mut Sink<'_, &mut [u8]>| {
            let first_pass = calls.fetch_add(1, Ordering::Relaxed) == 0;
            for _ in 0..if first_pass { 2 } else { 1 } {
                sink.push(0, 0);
            }
            Ok::<_, MemoryError>(())
        };
        let _ = scatter(&Memory::unlimited(), "x", 1, grain(1), emit, [&mut out[..]]);
    }

    /// An emitter that fails, in either pass, ends the call with the error of the first
    /// failed block, without the checks that only a complete pass can pass.
    #[test]
    fn scatter_returns_the_first_error_of_the_emitter() {
        #[derive(Debug, PartialEq)]
        enum Failure {
            Block(usize),
            Memory,
        }
        impl From<MemoryError> for Failure {
            fn from(_: MemoryError) -> Self {
                Self::Memory
            }
        }
        for failing_pass in [0_u64, 1] {
            let passes: Vec<AtomicU64> = (0..6).map(|_| AtomicU64::new(0)).collect();
            let mut out = [0_u8; 6];
            let emit = |block: usize, sink: &mut Sink<'_, &mut [u8]>| {
                let pass = passes[block].fetch_add(1, Ordering::Relaxed);
                assert_eq!(sink.is_counting(), pass == 0);
                if block >= 3 && pass == failing_pass {
                    // Half of the block was sent: the second pass would see too few.
                    return Err(Failure::Block(block));
                }
                sink.push(block % 2, 0);
                Ok(())
            };
            let got = scatter(&Memory::unlimited(), "x", 6, grain(2), emit, [&mut out[..]]);
            assert_eq!(
                got,
                Err(Failure::Block(3)),
                "failing in pass {failing_pass}"
            );
            let calls: Vec<u64> = passes.iter().map(|p| p.load(Ordering::Relaxed)).collect();
            let expected = if failing_pass == 0 { 1 } else { 2 };
            assert!(calls.iter().all(|&c| c == expected), "{calls:?}");
        }
    }

    /// The acceptance property of issue #189: each write helper gives the same bytes at
    /// 1, 3 and 8 threads, flat and segmented, equal to the sequential model.
    #[cfg(feature = "parallel")]
    #[test]
    fn writes_do_not_depend_on_the_thread_count() {
        let mut rng = Lcg(3);
        let len = 96;
        let mut offsets: Vec<usize> = (0..24).map(|_| rng.below(len + 1)).collect();
        offsets.sort_unstable();
        // Every range fits a segment of 2^5 rows, and some cross a segment end.
        assert!(offsets.windows(2).all(|w| w[1] - w[0] <= 32));
        assert!(
            offsets
                .windows(2)
                .any(|w| w[1] > w[0] && w[0] / 32 < (w[1] - 1) / 32)
        );
        let items = items(8, len, 7);
        let mut results = Vec::new();
        for threads in [1, 3, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            results.push(pool.install(|| {
                assert_eq!(rayon::current_num_threads(), threads);
                let ranges = ranged(len, &offsets, 4, &[5]);
                (ranges, scattered_by_facade(&items, 7, 16, &[2]))
            }));
        }
        let (values, indices, _) = scattered(&items, 7);
        assert_eq!(results[0], (marked(len, &offsets), (values, indices)));
        assert!(results.iter().all(|result| *result == results[0]));
    }
}
