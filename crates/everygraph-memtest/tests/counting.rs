// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The counting allocator's counts, and every one of its `unsafe` blocks (CONTRIBUTING.md,
//! "`unsafe`": `cargo +nightly miri test -p everygraph-memtest --test counting`). One
//! test only: under `cargo test`, the threads of other tests would allocate and free
//! while this one counts.

#![forbid(unsafe_code)]

use std::sync::Arc;

use everygraph_core::{Memory, MemoryKind, Pages};
use everygraph_memtest::counting::{self, Counting};
use everygraph_memtest::ledger::Ledger;

#[global_allocator]
static HEAP: Counting = Counting;

#[test]
fn every_block_is_counted_and_each_phase_keeps_its_untracked_bytes() {
    assert!(counting::installed());
    let base = counting::start();
    let mut v: Vec<u8> = Vec::with_capacity(1000);
    assert_eq!(counting::live() - base, 1000);
    v.resize(1000, 1);
    v.reserve_exact(3000);
    // Growth by `realloc`: the old and the new block both count at the peak.
    assert_eq!(counting::live() - base, 4000);
    assert_eq!(counting::peak() - base, 5000);
    v.truncate(10);
    v.shrink_to_fit();
    assert_eq!(counting::live() - base, 10);
    let zeroed = vec![0_u64; 64];
    assert_eq!(counting::live() - base, 10 + 512);
    drop((v, zeroed));
    assert_eq!(counting::live(), base);

    let ledger = Arc::new(Ledger::new(Some(1 << 20), Pages::HEAP, Some(3)));
    let memory = Memory::from(Arc::clone(&ledger));
    counting::start();
    let before = vec![0_u8; 100];
    let held = memory
        .reserve("first", MemoryKind::Anonymous, 1000)
        .unwrap();
    let covered = vec![0_u8; 1000];
    let second = memory.reserve("second", MemoryKind::Anonymous, 10).unwrap();
    let beyond = vec![0_u8; 5000];
    let phases = counting::untracked_by_phase();
    let of = |name: &str| phases.iter().find(|p| p.0 == name).map(|p| p.1);
    assert_eq!(of("(before any reservation)"), Some(100));
    // Above the start: `before` (100) and `covered` (1000) beyond the 1000 reserved.
    assert_eq!(of("first"), Some(100));
    assert_eq!(of("second"), Some(100 + 5000 - 10));
    // The third reservation is refused, as asked, and recorded.
    assert!(memory.reserve("third", MemoryKind::Anonymous, 1).is_err());
    assert_eq!(
        (ledger.refused(), ledger.reservations()),
        (Some("third"), 3)
    );
    drop((before, held, covered, second, beyond));
    assert_eq!(ledger.budget().used(), 0);
}
