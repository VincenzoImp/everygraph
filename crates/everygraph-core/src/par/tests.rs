// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::test_support::Lcg;
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
