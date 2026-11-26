// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the compressed stream reads, counted at its `ByteSource`: a full scan reads the
//! data part about once, and a scan whose filter passes few nodes reads only their
//! records, at 1 and 4 threads and at the smallest and a larger window. The kernels on
//! it give the in-memory bits (`tests/oracle.rs`, `tests/kernels.rs`); here the
//! components of a graph with many of them do too, while the finish reads few records.

#![forbid(unsafe_code)]
// Test graphs are small buffers outside the budget; casts cannot truncate.
#![allow(clippy::disallowed_methods, clippy::cast_possible_truncation)]

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

use std::sync::Arc;

use everygraph_core::{Budget, ByteSource, Memory, MemorySource, ReadError, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, Encoded, encode};
use everygraph_kernels::components::{WccOptions, wcc};
use everygraph_kernels::edge_map::{
    Compressed, CompressedStream, FRONTIER_GRAIN, Node, Plan, Settings, Source,
};

/// A data part in memory that counts the bytes read from it.
struct Counted {
    bytes: MemorySource<Vec<u8>>,
    read: AtomicU64,
}

impl Counted {
    fn new(data: &[u8]) -> Self {
        Self {
            bytes: MemorySource::new(data.to_vec()),
            read: AtomicU64::new(0),
        }
    }

    /// Bytes read since the last call.
    fn take(&self) -> u64 {
        self.read.swap(0, Relaxed)
    }
}

impl ByteSource for Counted {
    fn len(&self) -> u64 {
        self.bytes.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        self.read.fetch_add(buf.len() as u64, Relaxed);
        self.bytes.read_at(offset, buf)
    }
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Whether `v` is in one of the small components: four nodes in every 256.
fn small(v: u32) -> bool {
    v % 256 < 4
}

/// A symmetric graph of `n` nodes: the small nodes in pairs (`v`, `v + 1`), the others
/// one component in which each draws 100 neighbours, so lists of about 200 entries.
fn components(n: u32) -> (Vec<u64>, Vec<u32>) {
    let mut s = 3;
    let mut pairs = Vec::new();
    for v in 0..n {
        if small(v) {
            if v % 2 == 0 && v + 1 < n {
                pairs.extend([(v, v + 1), (v + 1, v)]);
            }
            continue;
        }
        for _ in 0..100 {
            let u = (splitmix(&mut s) % u64::from(n)) as u32;
            if u != v && !small(u) {
                pairs.extend([(v, u), (u, v)]);
            }
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    let mut offsets = vec![0_u64; n as usize + 1];
    for &(v, _) in &pairs {
        offsets[v as usize + 1] += 1;
    }
    for v in 0..n as usize {
        offsets[v + 1] += offsets[v];
    }
    (offsets, pairs.iter().map(|p| p.1).collect())
}

fn encoded(n: u32) -> (Encoded, AdjacencyIndex) {
    let (offsets, targets) = components(n);
    let encoded = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    (encoded, index)
}

/// Bytes read by one scan of `stream` whose filter is `pass`, at `threads` threads.
fn scan_reads(
    stream: &CompressedStream<'_>,
    data: &Counted,
    threads: usize,
    pass: fn(u32) -> bool,
) -> u64 {
    let memory = Memory::unlimited();
    data.take();
    Plan::with(stream, &memory, stream.settings(), |plan| {
        par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
            plan.scan(
                (),
                |v: Node<'_>| pass(v.get()),
                |(): &mut (), (), _, _, _| (),
            )
            .unwrap();
        })
        .unwrap();
    });
    data.take()
}

/// One full scan reads at most 1.05 times the data part: the tail a window holds is
/// moved, not read again, and a task starts at most 4 KiB before its first record.
#[test]
fn a_full_scan_reads_the_data_part_about_once() {
    let (encoded, index) = encoded(20_000);
    let data = Counted::new(&encoded.data);
    let size = encoded.data.len() as f64;
    for window in [0, 64 << 10] {
        let stream = CompressedStream::new(&index, &data, window).unwrap();
        for threads in [1, 4] {
            let read = scan_reads(&stream, &data, threads, |_| true) as f64;
            assert!(
                read >= size && read <= 1.05 * size,
                "window {window}, {threads} threads: {read} B for {size} B"
            );
        }
    }
}

/// A filter that passes the small nodes, four in every 256, reads their records only:
/// at most 0.15 times the data part. With a full scan's read-ahead instead, a 64 KiB
/// window reads nearly all of it.
#[test]
fn a_sparse_filter_reads_only_the_records_of_the_nodes_that_pass() {
    let (encoded, index) = encoded(20_000);
    let data = Counted::new(&encoded.data);
    let size = encoded.data.len() as f64;
    for window in [0, 64 << 10] {
        let stream = CompressedStream::new(&index, &data, window).unwrap();
        for threads in [1, 4] {
            let read = scan_reads(&stream, &data, threads, small) as f64;
            assert!(
                read > 0.0 && read <= 0.15 * size,
                "window {window}, {threads} threads: {read} B for {size} B"
            );
        }
    }
}

/// Components with sampling: the finish skips the giant component, so it reads only the
/// small ones' records. The labels are those of the compressed format in memory; the
/// bytes read are those of one pass and at most 0.15 times more.
#[test]
fn components_read_one_pass_and_the_small_components() {
    let (encoded, index) = encoded(20_000);
    let memory = Memory::unlimited();
    let load = |cells: Vec<AtomicU32>| -> Vec<u32> {
        cells.into_iter().map(AtomicU32::into_inner).collect()
    };
    let options = WccOptions::default();
    let heap = Compressed::new(Adjacency::new(&index, &encoded.data).unwrap());
    let expected = Plan::with(&heap, &memory, Settings::default(), |plan| {
        load(wcc(&plan, &memory, options).unwrap().0)
    });
    assert!(expected.iter().zip(0..).filter(|&(&l, v)| l == v).count() > 100);
    let data = Counted::new(&encoded.data);
    let stream = CompressedStream::new(&index, &data, 0).unwrap();
    let size = encoded.data.len() as f64;
    for threads in [1, 4] {
        data.take();
        let labels = Plan::with(&stream, &memory, stream.settings(), |plan| {
            par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
                load(wcc(&plan, &memory, options).unwrap().0)
            })
            .unwrap()
        });
        let read = data.take() as f64;
        assert_eq!(labels, expected, "{threads} threads");
        assert!(
            read >= size && read <= 1.15 * size,
            "{threads} threads: {read} B for {size} B"
        );
    }
}

/// One reader that reads three hubs, each longer than the last, without going back to
/// the pool between them: it frees each grown buffer before it grows again, so its
/// reservations never pass [`CompressedStream::reader_bytes`].
#[test]
fn a_reader_never_holds_more_than_reader_bytes() {
    let n = 500_000_u32;
    let hubs = [300_000_u32, 350_000, 400_000];
    let (mut offsets, mut targets) = (vec![0_u64], Vec::new());
    for v in 0..n {
        let d = hubs.get(v as usize).copied().unwrap_or(0);
        targets.extend((0..d).map(|k| (v + 1 + k) % n));
        let at = offsets[v as usize] as usize;
        targets[at..].sort_unstable();
        offsets.push(targets.len() as u64);
    }
    let encoded = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    let data = MemorySource::new(&encoded.data[..]);
    let stream = CompressedStream::new(&index, &data, 0).unwrap();
    let batch = Settings::default().batch_entries;
    let budget = Arc::new(Budget::unlimited());
    let memory = Memory::from(budget.clone());
    // As a plan makes it: lists of a block or of a frontier run, whichever is more.
    let lists = (n.div_ceil(4096).next_power_of_two().clamp(64, 4096) as usize).max(FRONTIER_GRAIN);
    let mut reader = stream.reader(&memory, batch, lists).unwrap();
    for v in 0..3 {
        let (_, list) = stream.list(&mut reader, v).unwrap();
        assert_eq!(list.len(), hubs[v as usize] as usize);
    }
    let bound =
        CompressedStream::reader_bytes(n, index.entries(), index.data_bytes(), hubs[2], 0, batch);
    assert!(
        budget.peak() <= bound,
        "{} B reserved, {bound} B bound",
        budget.peak()
    );
}
