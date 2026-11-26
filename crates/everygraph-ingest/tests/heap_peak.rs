// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The heap a dictionary build uses, against the budget its accountant was given: the
//! peak of the bytes the program holds from the allocator, above what it held before
//! the build, must not exceed the budget. Spilled bytes go
//! to files, so they are not on the heap. And the heap of the text reader on zstd
//! frames that declare a large window (issue #191).
//!
//! This measures the heap as the program asks for it; the allocator's own retention on
//! top of it is issue `m2-19-allocator-policy`.
//!
//! Counting needs a global allocator, which needs `unsafe` (module `counting`,
//! CONTRIBUTING.md, "`unsafe`"). The cases here are small enough for CI; the large
//! sizes run with `cargo test --release -p everygraph-ingest --test heap_peak --
//! --ignored`.

#![cfg(unix)]

mod support;

use std::sync::Mutex;

use everygraph_core::Budget;
use everygraph_ingest::ids::{BuildOptions, ColumnType, IdOrder, Layout, build_dictionary};
use support::{Files, Synthetic, binary20, hex, short, sparse_int, url};

mod counting {
    //! A global allocator that passes everything to `System` and counts the bytes in
    //! use and their peak. Safe code cannot replace the global allocator, hence the
    //! module's `unsafe` (CONTRIBUTING.md, "`unsafe`").
    #![allow(unsafe_code)]

    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);

    /// Bytes in use now.
    pub fn live() -> usize {
        LIVE.load(Ordering::Relaxed)
    }

    /// Starts a new peak from the bytes in use now, and returns them.
    pub fn reset_peak() -> usize {
        let now = live();
        PEAK.store(now, Ordering::Relaxed);
        now
    }

    /// The largest number of bytes in use since [`reset_peak`].
    pub fn peak() -> usize {
        PEAK.load(Ordering::Relaxed)
    }

    fn grew(bytes: usize) {
        let now = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
        PEAK.fetch_max(now, Ordering::Relaxed);
    }

    pub struct Counting;

    // SAFETY: every block comes from `System` and goes back to it with the layout it
    // was allocated with; the counters do not touch the blocks.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller's contract for `alloc` is passed on unchanged.
            let p = unsafe { System.alloc(layout) };
            if !p.is_null() {
                grew(layout.size());
            }
            p
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: as for `alloc`.
            let p = unsafe { System.alloc_zeroed(layout) };
            if !p.is_null() {
                grew(layout.size());
            }
            p
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            // SAFETY: `ptr` was allocated by `System` with `layout` (every block of
            // this allocator is).
            unsafe { System.dealloc(ptr, layout) };
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // The old and the new block may both exist while the data moves.
            grew(new_size);
            // SAFETY: `ptr` was allocated by `System` with `layout`, and the caller
            // guarantees `new_size` is valid for it.
            let p = unsafe { System.realloc(ptr, layout, new_size) };
            if p.is_null() {
                LIVE.fetch_sub(new_size, Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            }
            p
        }
    }

    #[global_allocator]
    static GLOBAL: Counting = Counting;
}

/// Held while a test measures the peak: the tests of a binary share one allocator.
static MEASURING: Mutex<()> = Mutex::new(());

struct Case {
    name: &'static str,
    column: ColumnType,
    make: fn(u64, &mut Vec<u8>) -> Option<u64>,
    n: u64,
    budget: u64,
    order: IdOrder,
    layout: Layout,
}

/// Builds `case` and returns (heap peak above the start, ledger peak).
fn measure(case: &Case) -> (u64, u64) {
    let _measuring = MEASURING.lock().unwrap();
    let scratch = Files::new(std::env::temp_dir().join(format!(
        "everygraph-heap-{}-{}",
        std::process::id(),
        case.name
    )));
    let ledger = Budget::new(case.budget);
    let source = Synthetic {
        column: case.column,
        n: case.n,
        stream: case.n + case.n / 2,
        make: case.make,
    };
    let options = BuildOptions::default()
        .with_order(case.order)
        .with_layout(case.layout);
    let start = counting::reset_peak();
    let built = build_dictionary(&source, &options, &ledger, &scratch).unwrap();
    let heap = counting::peak() - start;
    assert!(!built.dictionary.is_empty());
    drop(built);
    drop(scratch);
    (u64::try_from(heap).unwrap(), ledger.peak())
}

fn check(cases: &[Case]) {
    // Threads exist before the first measurement (rayon's pool allocates once).
    everygraph_core::par::for_each_range(64, std::num::NonZeroUsize::MIN, |_| {});
    for case in cases {
        let (heap, ledger) = measure(case);
        #[expect(clippy::print_stderr, reason = "the ratios are the test's report")]
        {
            eprintln!(
                "{}: heap peak {heap} B, ledger peak {ledger} B, budget {} B: heap/budget {:.3}, heap/ledger {:.3}",
                case.name,
                case.budget,
                heap as f64 / case.budget as f64,
                heap as f64 / ledger.max(1) as f64
            );
        }
        assert!(
            heap <= case.budget,
            "{}: heap {heap} above the budget {}",
            case.name,
            case.budget
        );
    }
}

fn cases(scale: u64) -> Vec<Case> {
    let utf8 = ColumnType::Utf8;
    vec![
        Case {
            name: "short-sorted-split",
            column: utf8,
            make: short,
            n: 600_000 * scale,
            budget: (16 << 20) * scale,
            order: IdOrder::SortedKey,
            layout: Layout::Split,
        },
        Case {
            name: "short-sorted-slot",
            column: utf8,
            make: short,
            n: 600_000 * scale,
            budget: (16 << 20) * scale,
            order: IdOrder::SortedKey,
            layout: Layout::SlotOrdered,
        },
        Case {
            name: "short-first",
            column: utf8,
            make: short,
            n: 600_000 * scale,
            budget: (16 << 20) * scale,
            order: IdOrder::FirstAppearance,
            layout: Layout::Split,
        },
        Case {
            name: "hex",
            column: utf8,
            make: hex,
            n: 400_000 * scale,
            budget: (16 << 20) * scale,
            order: IdOrder::Natural,
            layout: Layout::Split,
        },
        Case {
            name: "url",
            column: utf8,
            make: url,
            n: 300_000 * scale,
            budget: (16 << 20) * scale,
            order: IdOrder::Natural,
            layout: Layout::SlotOrdered,
        },
        Case {
            name: "binary20",
            column: ColumnType::FixedBinary { width: 20 },
            make: binary20,
            n: 500_000 * scale,
            budget: (16 << 20) * scale,
            order: IdOrder::Natural,
            layout: Layout::Split,
        },
        Case {
            name: "sparse-int",
            column: ColumnType::UInt { bits: 64 },
            make: sparse_int,
            n: 500_000 * scale,
            budget: (16 << 20) * scale,
            order: IdOrder::Natural,
            layout: Layout::Split,
        },
    ]
}

#[test]
fn the_heap_stays_within_the_budget() {
    if cfg!(miri) {
        // Miri checks the allocator's `unsafe`, on a few hundred keys.
        let mut tiny = cases(1);
        tiny.truncate(2);
        for c in &mut tiny {
            c.n = 300;
        }
        check(&tiny);
    } else {
        check(&cases(1));
    }
}

/// Large sizes: 15.5 M short keys under 256 MiB, and the others scaled alike.
#[test]
#[ignore = "minutes in a release build: cargo test --release -p everygraph-ingest --test heap_peak -- --ignored"]
fn the_heap_stays_within_the_budget_at_large_sizes() {
    check(&cases(16));
}

/// A zstd frame (RFC 8878 § 3.1.1) holding `text` in one raw block, with a window
/// descriptor of 128 MiB (`0x88`: exponent 17, mantissa 0), the most `ruzstd` accepts.
fn frame_with_a_large_window(text: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0x00, 0x88];
    let header = 1 | (u32::try_from(text.len()).unwrap() << 3);
    frame.extend_from_slice(&header.to_le_bytes()[..3]);
    frame.extend_from_slice(text);
    frame
}

/// Bytes the heap gained at its peak while `f` ran.
fn peak_of(f: impl FnOnce()) -> usize {
    let start = counting::reset_peak();
    f();
    counting::peak() - start
}

/// `ruzstd` 0.9.1 reserves a frame's declared window when a decoder is reused for
/// another frame (`DecodeBuffer::reset`), not when a decoder is new: a reader that
/// reused decoders would request up to 128 MiB per frame decoded at once, outside the
/// ledger (issue #191). The reader makes a decoder per frame; this fails if it reuses
/// one.
#[test]
fn zstd_frames_that_declare_a_large_window_do_not_get_it() {
    use std::io::Read;

    use everygraph_core::MemorySource;
    use everygraph_ingest::text::{TextOptions, TextReader};
    use ruzstd::decoding::{FrameDecoder, StreamingDecoder};

    if cfg!(miri) {
        return;
    }
    let _measuring = MEASURING.lock().unwrap();
    let one = frame_with_a_large_window(b"1 2\n");
    // What ruzstd does, pinned: if this changes, the reasoning above is to be redone.
    let fresh = peak_of(|| {
        let mut text = Vec::new();
        StreamingDecoder::new(one.as_slice())
            .unwrap()
            .read_to_end(&mut text)
            .unwrap();
        assert_eq!(text, b"1 2\n");
    });
    let reused = peak_of(|| {
        let mut decoder = FrameDecoder::new();
        decoder.reset(one.as_slice()).unwrap();
        decoder.reset(one.as_slice()).unwrap();
    });
    assert!(fresh < 1 << 20, "a new decoder took {fresh} bytes");
    assert!(
        reused >= 128 << 20,
        "a reused decoder took only {reused} bytes"
    );
    // The reader, on many such frames, which it decodes in parallel groups.
    let frames: Vec<u8> = (0..64)
        .flat_map(|i| frame_with_a_large_window(format!("{i} {}\n", i + 1).as_bytes()))
        .collect();
    let mut rows = 0;
    let reader = peak_of(|| {
        let text = TextReader::open(MemorySource::new(frames), &TextOptions::default()).unwrap();
        rows = text.read(|batch| batch.len()).unwrap().summary.rows;
    });
    assert_eq!(rows, 64);
    assert!(reader < 16 << 20, "the reader took {reader} bytes");
}
