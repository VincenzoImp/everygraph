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

    let failure = FirstFailure(std::sync::Mutex::new(None));
    for_each_chunk_mut(&mut counts, buckets, |start, row| {
        let block = start / width;
        let mut sink = Sink {
            row: Row::Count(row),
        };
        if let Err(error) = emit(block, &mut sink) {
            failure.record(block, error);
        }
    });
    if let Some(error) = failure.take() {
        return Err(error);
    }

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
            failure.record(block, error);
            return;
        }
        assert!(
            row.iter().all(|cell| cell.rows() == 0) && more.is_empty(),
            "block {block} sent fewer items than in the first pass"
        );
    });
    match failure.take() {
        Some(error) => Err(error),
        None => Ok(offsets),
    }
}

/// The error of the first block, by block order, that failed in a parallel pass.
struct FirstFailure<X>(std::sync::Mutex<Option<(usize, X)>>);

impl<X> FirstFailure<X> {
    fn record(&self, block: usize, error: X) {
        let mut first = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if first.as_ref().is_none_or(|(b, _)| block < *b) {
            *first = Some((block, error));
        }
    }

    fn take(&self) -> Option<X> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .map(|(_, error)| error)
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
mod tests;
