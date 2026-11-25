// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: buffers sized outside the memory budget, in a kernel crate (issue #31).
//
// check: clippy
// crates: everygraph-kernels everygraph-community everygraph-map everygraph-query
// expect: use of a disallowed method `std::vec::Vec::with_capacity`
// expect: use of a disallowed method `std::vec::Vec::reserve`
// expect: use of a disallowed method `std::vec::Vec::reserve_exact`
// expect: use of a disallowed method `std::vec::Vec::try_reserve`
// expect: use of a disallowed method `std::vec::Vec::try_reserve_exact`
// expect: use of a disallowed method `std::vec::Vec::resize`
// expect: use of a disallowed method `std::vec::Vec::resize_with`
// expect: use of a disallowed method `std::vec::from_elem`
// expect: use of a disallowed method `std::boxed::Box::new_uninit_slice`
// expect: use of a disallowed method `std::collections::HashMap::with_capacity`
// expect: use of a disallowed method `std::collections::HashMap::with_capacity_and_hasher`
// expect: use of a disallowed method `std::collections::HashMap::reserve`
// expect: use of a disallowed method `std::collections::HashSet::with_capacity`
// expect: use of a disallowed method `std::collections::HashSet::reserve`
// expect: use of a disallowed method `std::collections::VecDeque::with_capacity`
// expect: use of a disallowed method `std::collections::VecDeque::reserve`

//! Allocation canary probe (never part of a build of the real crate).

use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet, VecDeque};

/// Sizes buffers by `n` in every way the rule forbids.
pub fn allocate(n: usize) -> usize {
    let mut a: Vec<u32> = Vec::with_capacity(n);
    a.reserve(n);
    a.reserve_exact(n);
    let _ = a.try_reserve(n);
    let _ = a.try_reserve_exact(n);
    a.resize(n, 0);
    a.resize_with(n, Default::default);
    let b = vec![0_u64; n];
    let c = Box::<[u8]>::new_uninit_slice(n);
    let mut d: HashMap<u32, u32> = HashMap::with_capacity(n);
    d.reserve(n);
    let e: HashMap<u32, u32, RandomState> =
        HashMap::with_capacity_and_hasher(n, RandomState::new());
    let mut f: HashSet<u32> = HashSet::with_capacity(n);
    f.reserve(n);
    let mut g: VecDeque<u32> = VecDeque::with_capacity(n);
    g.reserve(n);
    a.len() + b.len() + c.len() + d.len() + e.len() + f.len() + g.len()
}
