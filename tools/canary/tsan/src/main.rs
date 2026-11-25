// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A toy kernel with a data race, kept out of every normal build.
//!
//! It counts the edges of a small generated graph per node, the way a kernel would, but
//! every chunk of `everygraph_core::par::for_each_range` increments the same plain
//! (non-atomic) counters through a shared raw pointer. That is undefined behaviour,
//! which safe Rust would not let us write; `unsafe` here is the point. Under
//! ThreadSanitizer this program must be reported with "data race". If it is not, the
//! ThreadSanitizer job no longer proves anything about the code it checks.
//!
//! `everygraph-core` forbids `unsafe`, so a racy test cannot live there.

use std::num::NonZeroUsize;

/// A pointer that the compiler is told it may share between threads. It may not.
struct Racy(*mut u64);

// SAFETY: none. This is the bug the canary exists to have.
unsafe impl Sync for Racy {}
// SAFETY: none, as above.
unsafe impl Send for Racy {}

impl Racy {
    fn get(&self) -> *mut u64 {
        self.0
    }
}

fn main() {
    const NODES: usize = 16;
    const EDGES: usize = 1 << 16;
    let mut degrees = vec![0_u64; NODES];
    let shared = Racy(degrees.as_mut_ptr());
    let grain = NonZeroUsize::new(64).expect("non-zero");
    everygraph_core::par::for_each_range(EDGES, grain, |range| {
        for edge in range {
            let node = (edge * 7 + 3) % NODES;
            // SAFETY: none: every chunk writes the same counters without synchronization.
            unsafe { *shared.get().add(node) += 1 };
        }
    });
    let total: u64 = degrees.iter().sum();
    println!("edges counted: {total} of {EDGES} (a racy count may lose some)");
}
