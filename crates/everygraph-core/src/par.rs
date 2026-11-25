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

use std::error::Error;
use std::fmt;
use std::num::NonZeroUsize;
use std::ops::Range;

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
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

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
}
