// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::Arc;

use super::*;
use crate::memory::{Budget, InsufficientBudget};
use crate::test_support::Lcg;

fn budget(limit: u64) -> (Arc<Budget>, Memory) {
    let budget = Arc::new(Budget::new(limit));
    let memory = Memory::from(Arc::clone(&budget));
    (budget, memory)
}

/// The invariants every operation must keep: the reservation is the capacity in
/// bytes, segments but the last are full-sized, and the elements match `model`.
fn check<T: Copy + PartialEq + fmt::Debug>(vec: &SegVec<T>, model: &[T]) {
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
        let refused = SegVec::try_with_len(&memory, "pagerank ranks", len, 0.0_f32).unwrap_err();
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

/// Shrinking reserves the smaller buffer before it releases the old one, and keeps
/// the capacity when the budget has no room for both.
#[test]
fn shrinking_keeps_the_capacity_when_both_buffers_do_not_fit() {
    for (limit, capacity, peak) in [(60, 60, 60), (70, 10, 70)] {
        let (budget, memory) = budget(limit);
        let mut vec = SegVec::<u8>::with_segment_bits(&memory, "bytes", 6);
        vec.try_reserve(60).unwrap();
        vec.try_extend_from_slice(&[7; 10]).unwrap();
        vec.shrink_to_fit();
        assert_eq!(vec.capacity(), capacity, "budget {limit}");
        assert_eq!(budget.used(), vec.reserved_bytes());
        assert_eq!(budget.used(), u64::try_from(capacity).unwrap());
        assert_eq!(budget.peak(), peak, "budget {limit}");
        assert!(vec.iter().all(|&b| b == 7));
    }
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
                (start..end).filter(|i| i & 3 == 0 || *i == start).count()
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
