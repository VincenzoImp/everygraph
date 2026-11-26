// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Issue #263: random reads of one large file at 4 KiB-aligned offsets, by read size and
//! queue depth. Two engines (a `pread` thread pool with one thread per slot of the queue,
//! and io_uring: one ring kept `qd` deep, or `io_uring_r<N>`, N rings on N threads) and two modes (buffered with POSIX_FADV_RANDOM,
//! and O_DIRECT). Prints one JSON line per cell.
//!
//! Buffered cells read only pages that were not in the page cache when the offsets were
//! drawn (checked with `mincore` on a mapping of the file that is never touched), and no
//! page twice in one cell. After each buffered cell, POSIX_FADV_DONTNEED on the whole file
//! drops the clean pages the cell brought in. So every buffered read is a cache miss,
//! without `drop_caches`; the device's read bytes per cell confirm it.

use io_uring::{IoUring, opcode, types};
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};
use std::time::{Duration, Instant};

const PAGE: u64 = 4096;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

struct AlignedBuf {
    ptr: *mut u8,
    layout: Layout,
}
impl AlignedBuf {
    fn new(len: usize) -> Self {
        let layout = Layout::from_size_align(len, PAGE as usize).unwrap();
        // SAFETY: the layout has a non-zero size.
        let ptr = unsafe { alloc_zeroed(layout) };
        assert!(!ptr.is_null());
        AlignedBuf { ptr, layout }
    }
}
impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout.
        unsafe { dealloc(self.ptr, self.layout) }
    }
}
unsafe impl Send for AlignedBuf {}

#[derive(Clone, Copy, Default)]
struct Disk {
    reads: u64,
    rsect: u64,
    writes: u64,
    wsect: u64,
    ticks_ms: u64,
}
fn disk(dev: &str) -> Disk {
    let s = std::fs::read_to_string("/proc/diskstats").unwrap();
    for l in s.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() > 12 && f[2] == dev {
            let p = |i: usize| f[i].parse::<u64>().unwrap();
            return Disk { reads: p(3), rsect: p(5), writes: p(7), wsect: p(9), ticks_ms: p(12) };
        }
    }
    panic!("device {dev} not in /proc/diskstats");
}
fn disk_json(a: Disk, b: Disk, secs: f64) -> String {
    format!(
        "\"dev_reads\":{},\"dev_read_bytes\":{},\"dev_writes\":{},\"dev_write_bytes\":{},\"dev_util\":{:.4}",
        b.reads - a.reads,
        (b.rsect - a.rsect) * 512,
        b.writes - a.writes,
        (b.wsect - a.wsect) * 512,
        (b.ticks_ms - a.ticks_ms) as f64 / 1000.0 / secs
    )
}
fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg").unwrap().split_whitespace().next().unwrap().to_string()
}

struct Target {
    buffered: File,
    direct: File,
    map: *mut libc::c_void,
    pages: u64,
    used: Vec<u64>,
    rng: Rng,
}
impl Target {
    fn is_used(&self, p: u64) -> bool {
        self.used[(p / 64) as usize] >> (p % 64) & 1 == 1
    }
    fn set_used(&mut self, p: u64, v: bool) {
        let w = &mut self.used[(p / 64) as usize];
        if v { *w |= 1 << (p % 64) } else { *w &= !(1 << (p % 64)) }
    }
    fn resident(&self, p: u64, k: u64) -> bool {
        let mut vec = vec![0u8; k as usize];
        // SAFETY: the range lies inside the mapping; mincore does not touch the pages.
        let r = unsafe {
            libc::mincore((self.map as *mut u8).add((p * PAGE) as usize).cast(), (k * PAGE) as usize, vec.as_mut_ptr())
        };
        assert_eq!(r, 0);
        vec.iter().any(|b| b & 1 == 1)
    }
    /// Offsets for one cell. Buffered offsets are fresh (see the module docs) and are marked
    /// used here until `release`.
    fn offsets(&mut self, size: u64, n: usize, fresh: bool) -> (Vec<u64>, u64) {
        let k = size / PAGE;
        let mut out = Vec::with_capacity(n);
        let mut rejected = 0u64;
        while out.len() < n {
            let p = self.rng.next() % (self.pages - k + 1);
            if fresh {
                if (p..p + k).any(|q| self.is_used(q)) || self.resident(p, k) {
                    rejected += 1;
                    assert!(rejected < 50 * n as u64 + 1000, "file has run out of fresh pages");
                    continue;
                }
                for q in p..p + k {
                    self.set_used(q, true);
                }
            }
            out.push(p * PAGE);
        }
        (out, rejected)
    }
    fn release(&mut self, offs: &[u64], size: u64) {
        for &o in offs {
            for q in o / PAGE..(o + size) / PAGE {
                self.set_used(q, false);
            }
        }
    }
}

struct Run {
    done: usize,
    issued: usize,
    secs: f64,
    lat_ns: Vec<u32>,
}

fn run_pread(fd: i32, offs: &[u64], size: usize, qd: usize, limit: Duration) -> Run {
    let idx = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let bar = Barrier::new(qd + 1);
    let lats = Mutex::new(Vec::new());
    let mut start = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..qd {
            s.spawn(|| {
                let buf = AlignedBuf::new(size);
                let mut lat = Vec::with_capacity(1 << 16);
                bar.wait();
                while !stop.load(Ordering::Relaxed) {
                    let i = idx.fetch_add(1, Ordering::Relaxed);
                    if i >= offs.len() {
                        break;
                    }
                    let t = Instant::now();
                    // SAFETY: `buf` holds `size` bytes.
                    let r = unsafe { libc::pread(fd, buf.ptr.cast(), size, offs[i] as i64) };
                    assert_eq!(r, size as isize, "pread at {}", offs[i]);
                    lat.push(t.elapsed().as_nanos().min(u32::MAX as u128) as u32);
                }
                lats.lock().unwrap().extend(lat);
            });
        }
        bar.wait();
        start = Instant::now();
        while start.elapsed() < limit && idx.load(Ordering::Relaxed) < offs.len() {
            std::thread::sleep(Duration::from_millis(1));
        }
        stop.store(true, Ordering::Relaxed);
    });
    let secs = start.elapsed().as_secs_f64();
    let lat_ns = lats.into_inner().unwrap();
    Run { done: lat_ns.len(), issued: idx.load(Ordering::Relaxed).min(offs.len()), secs, lat_ns }
}

/// `rings` threads, each with its own ring kept `qd / rings` deep, take offsets from one
/// shared index. One ring is the plain io_uring engine.
fn run_uring(fd: i32, offs: &[u64], size: usize, qd: usize, limit: Duration, rings: usize) -> Run {
    let rings = rings.min(qd);
    let idx = AtomicUsize::new(0);
    let bar = Barrier::new(rings + 1);
    let lats = Mutex::new(Vec::new());
    let mut start = Instant::now();
    std::thread::scope(|s| {
        for r in 0..rings {
            let depth = qd / rings + usize::from(r < qd % rings);
            let (idx, bar, lats) = (&idx, &bar, &lats);
            s.spawn(move || {
                let mut ring = IoUring::new(depth.next_power_of_two().max(8) as u32).expect("io_uring setup");
                let bufs: Vec<AlignedBuf> = (0..depth).map(|_| AlignedBuf::new(size)).collect();
                let mut sent = vec![Instant::now(); depth];
                let mut lat = Vec::with_capacity(1 << 16);
                let take = || {
                    let i = idx.fetch_add(1, Ordering::Relaxed);
                    (i < offs.len()).then(|| offs[i])
                };
                let push = |ring: &mut IoUring, slot: usize, off: u64| {
                    let e = opcode::Read::new(types::Fd(fd), bufs[slot].ptr, size as u32)
                        .offset(off)
                        .build()
                        .user_data(slot as u64);
                    // SAFETY: the buffer of `slot` stays alive and unused until its completion.
                    unsafe { ring.submission().push(&e).expect("sq full") };
                };
                bar.wait();
                let t0 = Instant::now();
                let mut inflight = 0usize;
                for slot in 0..depth {
                    let Some(off) = take() else { break };
                    sent[slot] = Instant::now();
                    push(&mut ring, slot, off);
                    inflight += 1;
                }
                while inflight > 0 {
                    ring.submit_and_wait(1).expect("submit");
                    let refill = t0.elapsed() < limit;
                    let done: Vec<(usize, i32)> =
                        ring.completion().map(|c| (c.user_data() as usize, c.result())).collect();
                    for (slot, res) in done {
                        assert_eq!(res, size as i32, "io_uring read result");
                        lat.push(sent[slot].elapsed().as_nanos().min(u32::MAX as u128) as u32);
                        inflight -= 1;
                        if refill && let Some(off) = take() {
                            sent[slot] = Instant::now();
                            push(&mut ring, slot, off);
                            inflight += 1;
                        }
                    }
                }
                lats.lock().unwrap().extend(lat);
            });
        }
        bar.wait();
        start = Instant::now();
    });
    let secs = start.elapsed().as_secs_f64();
    let lat_ns = lats.into_inner().unwrap();
    Run { done: lat_ns.len(), issued: idx.load(Ordering::Relaxed).min(offs.len()), secs, lat_ns }
}

fn pct(v: &mut [u32], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let i = ((v.len() - 1) as f64 * q).round() as usize;
    *v.select_nth_unstable(i).1 as f64 / 1000.0
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let get = |k: &str, d: &str| -> String {
        a.iter().position(|x| x == k).map(|i| a[i + 1].clone()).unwrap_or(d.to_string())
    };
    let path = get("--file", "/home/tux/graphlab/data/ldbc-graphalytics/graphs/com-friendster/com-friendster.e");
    let dev = get("--dev", "nvme0n1");
    let secs: f64 = get("--secs", "1.5").parse().unwrap();
    let reps: usize = get("--reps", "3").parse().unwrap();
    let buf_cap: u64 = get("--buffered-cap-gib", "8").parse::<u64>().unwrap() << 30;
    let direct_cap: u64 = get("--direct-cap-gib", "16").parse::<u64>().unwrap() << 30;
    let sizes: Vec<u64> = get("--sizes", "4,16,64").split(',').map(|s| s.parse::<u64>().unwrap() * 1024).collect();
    let qds: Vec<usize> = get("--qds", "1,4,16,64,256").split(',').map(|s| s.parse().unwrap()).collect();
    let limit = Duration::from_secs_f64(secs);

    let buffered = File::open(&path).unwrap();
    let direct = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECT).open(&path).unwrap();
    let len = buffered.metadata().unwrap().len();
    // SAFETY: a read-only shared mapping used only for mincore; it is never dereferenced.
    let map = unsafe { libc::mmap(std::ptr::null_mut(), len as usize, libc::PROT_READ, libc::MAP_SHARED, buffered.as_raw_fd(), 0) };
    assert_ne!(map, libc::MAP_FAILED);
    // SAFETY: plain advice calls on our own descriptor and mapping.
    unsafe {
        libc::madvise(map, len as usize, libc::MADV_RANDOM);
        libc::posix_fadvise(buffered.as_raw_fd(), 0, 0, libc::POSIX_FADV_RANDOM);
    }
    let pages = len / PAGE;
    let mut t = Target { buffered, direct, map, pages, used: vec![0; (pages / 64 + 1) as usize], rng: Rng(0x9E37_79B9_7F4A_7C15) };
    let resident = {
        let mut v = vec![0u8; pages as usize];
        // SAFETY: the vector has one byte per page of the mapping.
        assert_eq!(unsafe { libc::mincore(map, (pages * PAGE) as usize, v.as_mut_ptr()) }, 0);
        v.iter().filter(|b| *b & 1 == 1).count()
    };

    // Idle sample: the device with none of our I/O.
    let (d0, i0) = (disk(&dev), Instant::now());
    std::thread::sleep(Duration::from_secs(10));
    let d1 = disk(&dev);
    println!(
        "{{\"kind\":\"idle\",\"file\":\"{path}\",\"file_bytes\":{len},\"resident_pages_at_start\":{resident},\"file_pages\":{pages},\"load1\":{},{}}}",
        loadavg(),
        disk_json(d0, d1, i0.elapsed().as_secs_f64())
    );

    // Warm-up of the device and code paths, not reported.
    let (w, _) = t.offsets(4096, 1 << 20, false);
    run_uring(t.direct.as_raw_fd(), &w, 4096, 64, Duration::from_secs(1), 1);

    let engines: Vec<String> = get("--engines", "pread,io_uring").split(',').map(String::from).collect();
    let mut variants = Vec::new();
    for mode in get("--modes", "buffered,direct").split(',') {
        for e in &engines {
            variants.push((e.as_str(), if mode == "direct" { "direct" } else { "buffered" }));
        }
    }
    let mut group = 0usize;
    for rep in 0..reps {
        for &size in &sizes {
            for &qd in &qds {
                for v in 0..variants.len() {
                    let (engine, mode) = variants[(v + group) % variants.len()];
                    let fresh = mode == "buffered";
                    let n = if fresh { (buf_cap / size).min(1_200_000) } else { direct_cap / size } as usize;
                    let (offs, rejected) = t.offsets(size, n, fresh);
                    let fd = if fresh { t.buffered.as_raw_fd() } else { t.direct.as_raw_fd() };
                    let (da, load) = (disk(&dev), loadavg());
                    let mut r = match engine {
                        "pread" => run_pread(fd, &offs, size as usize, qd, limit),
                        "io_uring" => run_uring(fd, &offs, size as usize, qd, limit, 1),
                        e => {
                            let rings = e.strip_prefix("io_uring_r").and_then(|r| r.parse().ok());
                            run_uring(fd, &offs, size as usize, qd, limit, rings.expect("engine: pread, io_uring or io_uring_r<N>"))
                        }
                    };
                    let db = disk(&dev);
                    if fresh {
                        t.release(&offs, size);
                        // SAFETY: advice on our own descriptor.
                        unsafe { libc::posix_fadvise(t.buffered.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
                    }
                    let iops = r.done as f64 / r.secs;
                    let mean = r.lat_ns.iter().map(|&x| x as f64).sum::<f64>() / r.done.max(1) as f64 / 1000.0;
                    println!(
                        "{{\"kind\":\"cell\",\"rep\":{rep},\"size_kib\":{},\"qd\":{qd},\"engine\":\"{engine}\",\"mode\":\"{mode}\",\"reads\":{},\"secs\":{:.4},\"iops\":{:.0},\"bytes_per_s\":{:.0},\"lat_mean_us\":{mean:.1},\"lat_p50_us\":{:.1},\"lat_p99_us\":{:.1},\"capped\":{},\"fresh_rejected\":{rejected},\"load1\":{load},{}}}",
                        size / 1024,
                        r.done,
                        r.secs,
                        iops,
                        iops * size as f64,
                        pct(&mut r.lat_ns, 0.5),
                        pct(&mut r.lat_ns, 0.99),
                        r.issued >= offs.len(),
                        disk_json(da, db, r.secs)
                    );
                }
                group += 1;
            }
        }
    }
    println!("{{\"kind\":\"done\"}}");
}
