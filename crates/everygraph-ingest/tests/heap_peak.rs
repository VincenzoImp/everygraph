// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The heap a dictionary build uses, against the budget its accountant was given: the
//! peak of the bytes the program holds from the allocator, above what it held before
//! the build, must not exceed the budget (review of PR #158, S5/S6). Spilled bytes go
//! to files, so they are not on the heap.
//!
//! This measures the heap as the program asks for it; the allocator's own retention on
//! top of it is issue `m2-19-allocator-policy`.
//!
//! Counting needs a global allocator, which needs `unsafe` (module `counting`,
//! CONTRIBUTING.md, "`unsafe`"). The cases here are small enough for CI; the review's
//! sizes run with `cargo test --release -p everygraph-ingest --test heap_peak --
//! --ignored`.

#![cfg(unix)]

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use everygraph_core::{ByteSource, MemoryAccountant, ReadError};
use everygraph_ingest::ids::{
    BuildOptions, ColumnType, DictionaryError, IdOrder, Key, KeySource, Layout, Scratch,
    build_dictionary,
};

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

/// A fixed budget (anonymous memory only), with its peak.
struct Ledger {
    total: u64,
    used: AtomicU64,
    peak: AtomicU64,
}

impl MemoryAccountant for Ledger {
    fn try_reserve(
        &self,
        purpose: &'static str,
        kind: everygraph_core::MemoryKind,
        bytes: u64,
    ) -> Result<(), everygraph_core::InsufficientBudget> {
        if kind != everygraph_core::MemoryKind::Anonymous {
            return Ok(());
        }
        let mut used = self.used.load(Ordering::Relaxed);
        loop {
            let Some(after) = used.checked_add(bytes).filter(|a| *a <= self.total) else {
                return Err(everygraph_core::InsufficientBudget::new(
                    purpose,
                    bytes,
                    self.total - used,
                ));
            };
            match self
                .used
                .compare_exchange_weak(used, after, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.peak.fetch_max(after, Ordering::Relaxed);
                    return Ok(());
                }
                Err(now) => used = now,
            }
        }
    }

    fn release(&self, _: &'static str, kind: everygraph_core::MemoryKind, bytes: u64) {
        if kind == everygraph_core::MemoryKind::Anonymous {
            self.used.fetch_sub(bytes, Ordering::Relaxed);
        }
    }

    fn available(&self) -> u64 {
        self.total - self.used.load(Ordering::Relaxed)
    }
}

/// Scratch files with no buffer of their own (the library's write buffers are its
/// own, and reserved).
struct Files {
    dir: PathBuf,
    next: AtomicU64,
}

struct Sealed {
    file: File,
    len: u64,
    path: PathBuf,
}

impl ByteSource for Sealed {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        self.file
            .read_exact_at(buf, offset)
            .map_err(|e| ReadError::Backend(Box::new(e)))
    }
}

impl Drop for Sealed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

struct Writer {
    file: File,
    path: PathBuf,
}

impl io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Scratch for Files {
    type Source = Sealed;
    type Writer = Writer;

    fn create(&self, _: &'static str) -> io::Result<Writer> {
        let path = self
            .dir
            .join(format!("{}", self.next.fetch_add(1, Ordering::Relaxed)));
        Ok(Writer {
            file: File::create(&path)?,
            path,
        })
    }

    fn seal(&self, w: Writer) -> io::Result<Sealed> {
        drop(w.file);
        let file = File::open(&w.path)?;
        let len = file.metadata()?.len();
        Ok(Sealed {
            file,
            len,
            path: w.path,
        })
    }
}

/// Keys made on demand (so that the input is not on the heap).
struct Synthetic {
    column: ColumnType,
    n: u64,
    stream: u64,
    make: fn(u64, &mut Vec<u8>) -> Option<u64>,
}

impl KeySource for Synthetic {
    fn column(&self) -> ColumnType {
        self.column
    }
    fn chunks(&self) -> usize {
        usize::try_from(self.stream.div_ceil(1 << 14)).unwrap()
    }
    fn read_chunk(
        &self,
        chunk: usize,
        f: &mut dyn FnMut(u64, Key<'_>) -> Result<(), DictionaryError>,
    ) -> Result<(), DictionaryError> {
        let start = u64::try_from(chunk).unwrap() << 14;
        let mut buf = Vec::with_capacity(128);
        for j in start..(start + (1 << 14)).min(self.stream) {
            let i = splitmix(j) % self.n;
            buf.clear();
            match (self.make)(i, &mut buf) {
                Some(v) => f(j, Key::UInt(v))?,
                None => f(j, Key::Bytes(&buf))?,
            }
        }
        Ok(())
    }
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn hex(i: u64, out: &mut Vec<u8>) -> Option<u64> {
    out.extend_from_slice(
        format!(
            "0x{:016x}{:016x}{:08x}",
            splitmix(i),
            splitmix(i + 1),
            splitmix(i + 2) >> 32
        )
        .as_bytes(),
    );
    None
}

fn short(i: u64, out: &mut Vec<u8>) -> Option<u64> {
    out.extend_from_slice(format!("u{i}").as_bytes());
    None
}

fn url(i: u64, out: &mut Vec<u8>) -> Option<u64> {
    out.extend_from_slice(
        format!("https://www.example-social-network.com/profiles/user-{i}/about").as_bytes(),
    );
    None
}

fn binary20(i: u64, out: &mut Vec<u8>) -> Option<u64> {
    for j in 0..3 {
        out.extend_from_slice(&splitmix(i * 3 + j).to_le_bytes());
    }
    out.truncate(20);
    None
}

fn sparse_int(i: u64, _: &mut Vec<u8>) -> Option<u64> {
    Some(splitmix(i))
}

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
    let dir = std::env::temp_dir().join(format!(
        "everygraph-heap-{}-{}",
        std::process::id(),
        case.name
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let scratch = Files {
        dir: dir.clone(),
        next: AtomicU64::new(0),
    };
    let ledger = Ledger {
        total: case.budget,
        used: AtomicU64::new(0),
        peak: AtomicU64::new(0),
    };
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
    std::fs::remove_dir_all(&dir).unwrap();
    let ledger_peak = ledger.peak.load(Ordering::Relaxed);
    (u64::try_from(heap).unwrap(), ledger_peak)
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

/// The review's sizes (15.5 M short keys under 256 MiB, and the others scaled alike).
#[test]
#[ignore = "minutes in a release build: cargo test --release -p everygraph-ingest --test heap_peak -- --ignored"]
fn the_heap_stays_within_the_budget_at_the_review_sizes() {
    check(&cases(16));
}
