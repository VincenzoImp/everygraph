//! Text edge list -> glab-csr-v0 under a configurable memory budget.
//!
//! Pass 1  parse text in parallel -> raw (u,v) u32 pairs appended to a *spill* (RAM chunks while
//!         the budget allows, then a temp file) + concurrent id bitmap (ids must be < 2^32).
//! Rank    bitmap -> rank directory (dense id = rank of original id), ids.u64.
//! Pass 2  relabel pairs, emit both directions into B buckets partitioned by dense source range
//!         (thread-local write-combining buffers; buckets in RAM or one temp file each).
//! Pass 3  per bucket in parallel: counting sort by source, sort+dedup each adjacency list,
//!         ordered commit of targets with pwrite; degrees -> offsets.
use crate::{as_bytes, as_bytes_mut, peak_rss, SyncPtr};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Instant;

pub enum Input {
    /// seekable plain text file, read with parallel pread
    File(PathBuf),
    /// plain text on stdin (e.g. `pigz -dc x.gz | glab ingest --input -`)
    Stdin,
    /// gzip file decoded in-process (flate2 + zlib-rs), single decode thread
    GzInProc(PathBuf),
}

pub struct Config {
    pub input: Input,
    pub out_dir: PathBuf,
    pub tmp_dir: PathBuf,
    pub mem_budget: u64,
    pub threads: usize,
    pub fsync: bool,
    pub parse_only: bool,
    /// target raw bucket size in MiB (cache-sizing of the per-bucket sort)
    pub bucket_mb: u64,
    /// write targets through a shared file mapping instead of pwrite (avoids the per-inode write lock)
    pub out_mmap: bool,
    /// read the input text with O_DIRECT (no page-cache pollution, cold-cache friendly)
    pub direct_in: bool,
    /// O_DIRECT for temp files (spill + buckets): bypass the page cache when working out of core
    pub direct_tmp: bool,
    /// fallocate bucket temp files (2x the mean size) so interleaved appends stay contiguous
    pub prealloc: bool,
}

const TEXT_CHUNK: usize = 4 << 20;
const SLACK: usize = 64 << 10;
const SPILL_CAP: usize = 2 << 20; // u32s per spill chunk (8 MiB)

struct Bitmap {
    map: memmap2::MmapMut,
}
impl Bitmap {
    fn new() -> Bitmap {
        // 2^32 bits = 512 MiB of *virtual* zero pages; only touched pages are committed.
        Bitmap { map: memmap2::MmapMut::map_anon(1 << 29).expect("bitmap") }
    }
    #[inline(always)]
    fn words(&self) -> &[AtomicU64] {
        unsafe { std::slice::from_raw_parts(self.map.as_ptr() as *const AtomicU64, 1 << 26) }
    }
}

struct Spill {
    ram: Mutex<Vec<Vec<u32>>>,
    ram_bytes: AtomicU64,
    ram_limit: u64,
    file: File,
    path: PathBuf,
    file_len: AtomicU64,
    segs: Mutex<Vec<(u64, u64)>>,
    direct: bool,
}
thread_local! {
    static SCRATCH: std::cell::RefCell<Option<crate::extmem::AlignedBuf>> = const { std::cell::RefCell::new(None) };
}
/// O_DIRECT write of `data` (copied into a page-aligned scratch buffer, length rounded up to 4 KiB).
fn dio_write(f: &File, data: &[u8], off: u64) {
    SCRATCH.with(|c| {
        let mut c = c.borrow_mut();
        let need = data.len().next_multiple_of(4096);
        if c.as_ref().map(|b| b.cap() < need).unwrap_or(true) {
            *c = Some(crate::extmem::AlignedBuf::new(need.max(8 << 20)));
        }
        let sb = c.as_mut().unwrap().slice_mut();
        sb[..data.len()].copy_from_slice(data);
        f.write_all_at(&sb[..need], off).expect("O_DIRECT write");
    })
}
impl Spill {
    fn push(&self, buf: &mut Vec<u32>) {
        let bytes = (buf.len() * 4) as u64;
        if bytes == 0 {
            return;
        }
        let prev = self.ram_bytes.fetch_add(bytes, Relaxed);
        if prev + bytes <= self.ram_limit {
            let full = std::mem::replace(buf, Vec::with_capacity(SPILL_CAP));
            self.ram.lock().unwrap().push(full);
        } else {
            self.ram_bytes.fetch_sub(bytes, Relaxed);
            let off = self.file_len.fetch_add(bytes.next_multiple_of(4096), Relaxed);
            if self.direct {
                dio_write(&self.file, as_bytes(buf), off);
            } else {
                self.file.write_all_at(as_bytes(buf), off).expect("spill write");
            }
            self.segs.lock().unwrap().push((off, bytes));
            buf.clear();
        }
    }
}

struct P1Shared<'a> {
    bitmap: &'a Bitmap,
    spill: &'a Spill,
    parse_only: bool,
}
#[derive(Default)]
struct P1Worker {
    out: Vec<u32>,
    edges: u64,
    max_id: u64,
    last_a: u64,
    t_read: f64,
    t_parse: f64,
    bytes: u64,
}

#[inline(always)]
fn mark(words: &[AtomicU64], id: u64) {
    let w = unsafe { words.get_unchecked((id >> 6) as usize) };
    let bit = 1u64 << (id & 63);
    if w.load(Relaxed) & bit == 0 {
        w.fetch_or(bit, Relaxed);
    }
}

/// Parse a block of complete lines. `buf` must end with b'\n' (sentinel for the unchecked loops).
fn parse_block(buf: &[u8], w: &mut P1Worker, sh: &P1Shared) -> Result<(), String> {
    if buf.is_empty() {
        return Ok(());
    }
    assert_eq!(*buf.last().unwrap(), b'\n');
    let words = sh.bitmap.words();
    let mut edges = 0u64;
    let mut max_id = w.max_id;
    let mut last_a = w.last_a;
    unsafe {
        let mut p = buf.as_ptr();
        let end = p.add(buf.len());
        while p < end {
            let c = *p;
            let mut d = c.wrapping_sub(b'0');
            if d > 9 {
                if c == b'\n' || c == b' ' || c == b'\t' || c == b'\r' {
                    p = p.add(1);
                    continue;
                }
                if c == b'#' || c == b'%' {
                    while *p != b'\n' {
                        p = p.add(1);
                    }
                    p = p.add(1);
                    continue;
                }
                return Err(format!("unexpected byte 0x{c:02x} at start of field"));
            }
            let mut a = 0u64;
            while d <= 9 {
                a = a.wrapping_mul(10).wrapping_add(d as u64);
                p = p.add(1);
                d = (*p).wrapping_sub(b'0');
            }
            while *p == b'\t' || *p == b' ' || *p == b',' {
                p = p.add(1);
            }
            d = (*p).wrapping_sub(b'0');
            if d > 9 {
                return Err("line with a single column".into());
            }
            let mut b = 0u64;
            while d <= 9 {
                b = b.wrapping_mul(10).wrapping_add(d as u64);
                p = p.add(1);
                d = (*p).wrapping_sub(b'0');
            }
            while *p != b'\n' {
                p = p.add(1);
            }
            p = p.add(1);
            if (a | b) >> 32 != 0 {
                return Err(format!("id >= 2^32 ({a} {b}): prototype supports u32 original ids only"));
            }
            if a != last_a {
                mark(words, a);
                last_a = a;
            }
            mark(words, b);
            max_id = max_id.max(a).max(b);
            edges += 1;
            if !sh.parse_only {
                if w.out.len() + 2 > w.out.capacity() {
                    sh.spill.push(&mut w.out);
                }
                w.out.push(a as u32);
                w.out.push(b as u32);
            }
        }
    }
    w.edges += edges;
    w.max_id = max_id;
    w.last_a = last_a;
    Ok(())
}

fn new_worker() -> P1Worker {
    P1Worker { out: Vec::with_capacity(SPILL_CAP), last_a: u64::MAX, ..Default::default() }
}

fn pass1_file(path: &PathBuf, threads: usize, sh: &P1Shared, direct: bool) -> Result<Vec<P1Worker>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut oo = File::options();
    oo.read(true);
    if direct {
        oo.custom_flags(libc::O_DIRECT);
    }
    let f = oo.open(path).map_err(|e| format!("{path:?}: {e}"))?;
    let flen = f.metadata().map_err(|e| e.to_string())?.len() as usize;
    let nchunks = flen.div_ceil(TEXT_CHUNK);
    let next = AtomicUsize::new(0);
    let res: Vec<Result<P1Worker, String>> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| -> Result<P1Worker, String> {
                    let mut w = new_worker();
                    // page-aligned buffer and page-aligned file offsets: the same code serves O_DIRECT
                    let mut abuf = crate::extmem::AlignedBuf::new(4096 + TEXT_CHUNK + SLACK + 4096);
                    let full = abuf.slice_mut();
                    loop {
                        let c = next.fetch_add(1, Relaxed);
                        if c >= nchunks {
                            break;
                        }
                        let off = c * TEXT_CHUNK;
                        let end = (off + TEXT_CHUNK).min(flen);
                        let (astart, pre) = if off == 0 { (0, 0) } else { (off - 4096, 4095) };
                        let rstart = astart + pre; // = off - 1 for off > 0
                        let rlen = (end + SLACK).min(flen) - rstart;
                        let want = pre + rlen;
                        let t0 = Instant::now();
                        let mut got = 0usize;
                        while got < want {
                            let hi = (want.next_multiple_of(4096)).min(full.len());
                            let k = f.read_at(&mut full[got..hi], (astart + got) as u64).map_err(|e| e.to_string())?;
                            if k == 0 {
                                return Err("unexpected EOF".into());
                            }
                            got += k;
                        }
                        let t1 = Instant::now();
                        let buf = &mut full[pre..];
                        let s0 = if off == 0 {
                            0
                        } else {
                            match memchr::memchr(b'\n', &buf[..rlen]) {
                                Some(p) => p + 1,
                                None => rlen,
                            }
                        };
                        let e0 = end - 1 - rstart;
                        let e = if end == flen {
                            if buf[rlen - 1] != b'\n' {
                                buf[rlen] = b'\n';
                                rlen + 1
                            } else {
                                rlen
                            }
                        } else {
                            match memchr::memchr(b'\n', &buf[e0..rlen]) {
                                Some(p) => e0 + p + 1,
                                None => return Err("line longer than 64 KiB".into()),
                            }
                        };
                        if s0 < e {
                            parse_block(&buf[s0..e], &mut w, sh)?;
                        }
                        w.t_read += (t1 - t0).as_secs_f64();
                        w.t_parse += t1.elapsed().as_secs_f64();
                        w.bytes += (end - off) as u64;
                    }
                    sh.spill.push(&mut w.out);
                    Ok(w)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    res.into_iter().collect()
}

/// Streaming input: one reader thread cuts the stream into newline-terminated blocks.
/// Returns (workers, seconds the reader spent inside read(), seconds blocked on a full queue).
fn pass1_stream<R: Read>(mut r: R, threads: usize, sh: &P1Shared) -> Result<(Vec<P1Worker>, f64, f64), String> {
    let (tx, rx) = crossbeam_channel::bounded::<(Vec<u8>, usize)>(threads * 2);
    let (pool_tx, pool_rx) = crossbeam_channel::unbounded::<Vec<u8>>();
    std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                let rx = rx.clone();
                let pool_tx = pool_tx.clone();
                s.spawn(move || -> Result<P1Worker, String> {
                    let mut w = new_worker();
                    while let Ok((buf, len)) = rx.recv() {
                        let t1 = Instant::now();
                        parse_block(&buf[..len], &mut w, sh)?;
                        w.t_parse += t1.elapsed().as_secs_f64();
                        w.bytes += len as u64;
                        let _ = pool_tx.send(buf);
                    }
                    sh.spill.push(&mut w.out);
                    Ok(w)
                })
            })
            .collect();
        drop(rx);
        let mut t_read = 0f64;
        let mut t_send = 0f64;
        let mut carry: Vec<u8> = Vec::new();
        let mut err = None;
        loop {
            let mut buf = pool_rx.try_recv().unwrap_or_else(|_| vec![0u8; TEXT_CHUNK + 1]);
            let mut len = carry.len();
            if len >= TEXT_CHUNK {
                err = Some("line longer than 4 MiB".to_string());
                break;
            }
            buf[..len].copy_from_slice(&carry);
            let mut eof = false;
            let t0 = Instant::now();
            while len < TEXT_CHUNK {
                match r.read(&mut buf[len..TEXT_CHUNK]) {
                    Ok(0) => {
                        eof = true;
                        break;
                    }
                    Ok(k) => len += k,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        err = Some(e.to_string());
                        eof = true;
                        break;
                    }
                }
            }
            let t1 = Instant::now();
            t_read += (t1 - t0).as_secs_f64();
            if eof {
                if len > 0 {
                    if buf[len - 1] != b'\n' {
                        buf[len] = b'\n';
                        len += 1;
                    }
                    let _ = tx.send((buf, len));
                }
                break;
            }
            let cut = match memchr::memrchr(b'\n', &buf[..len]) {
                Some(p) => p + 1,
                None => {
                    err = Some("line longer than 4 MiB".to_string());
                    break;
                }
            };
            carry.clear();
            carry.extend_from_slice(&buf[cut..len]);
            if tx.send((buf, cut)).is_err() {
                break; // a worker failed
            }
            t_send += t1.elapsed().as_secs_f64();
        }
        drop(tx);
        let ws: Result<Vec<P1Worker>, String> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        if let Some(e) = err {
            return Err(e);
        }
        Ok((ws?, t_read, t_send))
    })
}

fn open_tmp(p: &PathBuf, direct: bool) -> Result<File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut o = File::options();
    o.read(true).write(true).create(true).truncate(true);
    if direct {
        o.custom_flags(libc::O_DIRECT);
    }
    o.open(p).map_err(|e| format!("{p:?}: {e}"))
}

static P2_READ_NS: AtomicU64 = AtomicU64::new(0);
static P2_WRITE_NS: AtomicU64 = AtomicU64::new(0);
static P2_TOTAL_NS: AtomicU64 = AtomicU64::new(0);

struct Buckets {
    on_disk: bool,
    direct: bool,
    /// buffered handles of the same files, used for the unaligned tails in O_DIRECT mode
    tails: Vec<File>,
    /// bytes written with O_DIRECT (page multiple) per bucket; the rest are tails
    dio_len: Vec<u64>,
    files: Vec<File>,
    paths: Vec<PathBuf>,
    lens: Vec<AtomicU64>,
    ram: Vec<Mutex<Vec<Vec<u32>>>>,
}
impl Buckets {
    #[inline]
    fn flush(&self, k: usize, v: &mut Vec<u32>, cap: usize) {
        if v.is_empty() {
            return;
        }
        let bytes = (v.len() * 4) as u64;
        let off = self.lens[k].fetch_add(bytes, Relaxed);
        if self.on_disk {
            let t = Instant::now();
            if self.direct {
                dio_write(&self.files[k], as_bytes(v), off);
            } else {
                self.files[k].write_all_at(as_bytes(v), off).expect("bucket write");
            }
            P2_WRITE_NS.fetch_add(t.elapsed().as_nanos() as u64, Relaxed);
            v.clear();
        } else {
            let full = std::mem::replace(v, Vec::with_capacity(cap));
            self.ram[k].lock().unwrap().push(full);
        }
    }
}

struct Commit {
    next: usize,
    base: u64, // entries committed so far
    pending: BTreeMap<usize, Vec<u32>>,
}

pub fn run(cfg: &Config) -> Result<serde_json::Value, String> {
    let t_all = Instant::now();
    crate::raise_nofile();
    unsafe {
        // big buffers are mmapped and really returned to the OS when dropped
        libc::mallopt(libc::M_MMAP_THRESHOLD, 1 << 20);
    }
    std::fs::create_dir_all(&cfg.out_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&cfg.tmp_dir).map_err(|e| e.to_string())?;
    let threads = cfg.threads.max(1);
    let budget = cfg.mem_budget;

    // ---------------- pass 1: parse ----------------
    let bitmap = Bitmap::new();
    let spill_path = cfg.tmp_dir.join(format!("spill.{}.bin", std::process::id()));
    let spill = Spill {
        ram: Mutex::new(Vec::new()),
        ram_bytes: AtomicU64::new(0),
        ram_limit: (budget as f64 * 0.40) as u64,
        file: open_tmp(&spill_path, cfg.direct_tmp)?,
        direct: cfg.direct_tmp,
        path: spill_path,
        file_len: AtomicU64::new(0),
        segs: Mutex::new(Vec::new()),
    };
    let sh = P1Shared { bitmap: &bitmap, spill: &spill, parse_only: cfg.parse_only };
    let (workers, t_reader_read, t_reader_blocked, mode) = match &cfg.input {
        Input::File(p) => (pass1_file(p, threads, &sh, cfg.direct_in)?, 0.0, 0.0, if cfg.direct_in { "file-odirect" } else { "file-pread" }),
        Input::Stdin => {
            use std::os::unix::io::FromRawFd;
            unsafe {
                libc::fcntl(0, libc::F_SETPIPE_SZ, 1 << 20);
            }
            // raw fd 0: no extra buffering layer between the pipe and our 4 MiB blocks
            let stdin = std::mem::ManuallyDrop::new(unsafe { File::from_raw_fd(0) });
            let (w, a, b) = pass1_stream(&*stdin, threads, &sh)?;
            (w, a, b, "stdin-pipe")
        }
        Input::GzInProc(p) => {
            let f = File::open(p).map_err(|e| format!("{p:?}: {e}"))?;
            let dec = flate2::read::MultiGzDecoder::new(std::io::BufReader::with_capacity(1 << 20, f));
            let (w, a, b) = pass1_stream(dec, threads, &sh)?;
            (w, a, b, "gz-inprocess")
        }
    };
    let t_pass1 = t_all.elapsed().as_secs_f64();
    let edges_parsed: u64 = workers.iter().map(|w| w.edges).sum();
    let text_bytes: u64 = workers.iter().map(|w| w.bytes).sum();
    let max_id: u64 = workers.iter().map(|w| w.max_id).max().unwrap_or(0);
    let p1_read_cpu: f64 = workers.iter().map(|w| w.t_read).sum();
    let p1_parse_cpu: f64 = workers.iter().map(|w| w.t_parse).sum();
    drop(workers);
    let spill_ram_bytes = spill.ram_bytes.load(Relaxed);
    let spill_disk_bytes = spill.file_len.load(Relaxed);
    let rss_after_p1 = crate::cur_rss();
    if cfg.parse_only {
        let _ = std::fs::remove_file(&spill.path);
        return Ok(json!({
            "mode": mode, "parse_only": true, "threads": threads, "edges_parsed": edges_parsed,
            "text_bytes": text_bytes, "seconds": t_pass1, "text_gb_per_s": text_bytes as f64 / 1e9 / t_pass1,
            "edges_per_s": edges_parsed as f64 / t_pass1,
            "p1_read_thread_seconds": p1_read_cpu, "p1_parse_thread_seconds": p1_parse_cpu,
            "reader_seconds_in_read": t_reader_read, "reader_seconds_blocked_on_parsers": t_reader_blocked,
        }));
    }

    // ---------------- rank directory ----------------
    let t0 = Instant::now();
    let words = bitmap.words();
    let nwords = if edges_parsed == 0 { 0 } else { (max_id >> 6) as usize + 1 };
    let mut cum: Vec<[u64; 2]> = Vec::with_capacity(nwords + 1);
    let mut acc = 0u64;
    for w in &words[..nwords] {
        let x = w.load(Relaxed);
        cum.push([x, acc]);
        acc += x.count_ones() as u64;
    }
    cum.push([0, acc]);
    if acc > u32::MAX as u64 {
        return Err("more than 2^32-1 nodes".into());
    }
    let n = acc as usize;
    let t_rank = t0.elapsed().as_secs_f64();
    let rank = |id: u32| -> u32 {
        unsafe {
            let e = cum.get_unchecked((id >> 6) as usize);
            e[1] as u32 + (e[0] & ((1u64 << (id & 63)) - 1)).count_ones()
        }
    };

    // ---------------- pass 2: relabel + partition ----------------
    let t0 = Instant::now();
    let bucket_bytes_total = edges_parsed * 16;
    let work_mem = 24.0 * edges_parsed as f64; // bytes if everything were one bucket
    let b_mem = work_mem * threads as f64 / (0.4 * budget as f64);
    let b_cache = bucket_bytes_total as f64 / (cfg.bucket_mb.max(1) << 20) as f64;
    let b_target = (b_mem.max(b_cache).max((4 * threads) as f64).max(1.0) as usize).next_power_of_two().min(2048);
    let nbits = if n <= 1 { 0 } else { usize::BITS - (n - 1).leading_zeros() };
    let shift = nbits.saturating_sub(b_target.trailing_zeros());
    let nb = if n == 0 { 1 } else { ((n - 1) >> shift) + 1 };
    let buckets_on_disk = (bucket_bytes_total + spill_ram_bytes + (512 << 20)) as f64 > 0.9 * budget as f64;
    let bufcap = {
        let per = (budget as f64 * 0.10 / (threads * nb) as f64) as usize / 4;
        let c = per.clamp(1024, 8192);
        1usize << (usize::BITS - 1 - c.leading_zeros()) // u32s, power of two => page-multiple bytes
    };
    let direct_tmp = cfg.direct_tmp && buckets_on_disk;
    let mut bk = Buckets { on_disk: buckets_on_disk, direct: direct_tmp, tails: vec![], dio_len: vec![], files: vec![], paths: vec![], lens: vec![], ram: vec![] };
    for b in 0..nb {
        bk.lens.push(AtomicU64::new(0));
        bk.ram.push(Mutex::new(Vec::new()));
        if buckets_on_disk {
            let p = cfg.tmp_dir.join(format!("bucket.{}.{b:05}.bin", std::process::id()));
            bk.files.push(open_tmp(&p, direct_tmp)?);
            if cfg.prealloc {
                use std::os::unix::io::AsRawFd;
                let est = (2 * bucket_bytes_total / nb as u64).next_multiple_of(1 << 20) as i64;
                unsafe { libc::fallocate(bk.files[b].as_raw_fd(), libc::FALLOC_FL_KEEP_SIZE, 0, est) };
            }
            if direct_tmp {
                bk.tails.push(File::options().read(true).write(true).open(&p).map_err(|e| e.to_string())?);
            }
            bk.paths.push(p);
        }
    }
    let ram_chunks: Mutex<Vec<Vec<u32>>> = Mutex::new(std::mem::take(&mut *spill.ram.lock().unwrap()));
    let segs: Vec<(u64, u64)> = std::mem::take(&mut *spill.segs.lock().unwrap());
    let next_seg = AtomicUsize::new(0);
    let ids_path = cfg.out_dir.join("ids.u64");
    let mut t_ids = 0.0;
    let mut all_tails: Vec<Vec<Vec<u32>>> = Vec::new();
    std::thread::scope(|s| {
        // ids.u64 writer runs concurrently with the partition pass
        let h_ids = s.spawn(|| {
            let t = Instant::now();
            let mut f = std::io::BufWriter::with_capacity(8 << 20, File::create(&ids_path).expect("ids"));
            use std::io::Write;
            for (wi, w) in words[..nwords].iter().enumerate() {
                let mut x = w.load(Relaxed);
                while x != 0 {
                    let id = ((wi as u64) << 6) | x.trailing_zeros() as u64;
                    f.write_all(&id.to_le_bytes()).unwrap();
                    x &= x - 1;
                }
            }
            f.flush().unwrap();
            t.elapsed().as_secs_f64()
        });
        let bk = &bk;
        let rank = &rank;
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut tl: Vec<Vec<u32>> = (0..nb).map(|_| Vec::with_capacity(bufcap)).collect();
                    let mut rbuf: Vec<u32> = Vec::new();
                    let mut abuf: Option<crate::extmem::AlignedBuf> = None;
                    let t_thread = Instant::now();
                    let mut t_rd = 0f64;
                    let process = |chunk: &[u32], tl: &mut Vec<Vec<u32>>| {
                        const PF: usize = 24; // prefetch distance in u32s (12 pairs ahead)
                        for (i, pr) in chunk.chunks_exact(2).enumerate() {
                            let (a, b) = (pr[0], pr[1]);
                            if 2 * i + PF + 1 < chunk.len() {
                                unsafe {
                                    use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
                                    _mm_prefetch(cum.as_ptr().add((chunk[2 * i + PF] >> 6) as usize) as *const i8, _MM_HINT_T0);
                                    _mm_prefetch(cum.as_ptr().add((chunk[2 * i + PF + 1] >> 6) as usize) as *const i8, _MM_HINT_T0);
                                }
                            }
                            if a == b {
                                continue;
                            }
                            let da = rank(a);
                            let db = rank(b);
                            let ka = (da >> shift) as usize;
                            let kb = (db >> shift) as usize;
                            unsafe {
                                let v = tl.get_unchecked_mut(ka);
                                v.push(da);
                                v.push(db);
                                if v.len() >= bufcap {
                                    bk.flush(ka, v, bufcap);
                                }
                                let v = tl.get_unchecked_mut(kb);
                                v.push(db);
                                v.push(da);
                                if v.len() >= bufcap {
                                    bk.flush(kb, v, bufcap);
                                }
                            }
                        }
                    };
                    loop {
                        let c = ram_chunks.lock().unwrap().pop();
                        match c {
                            Some(c) => process(&c, &mut tl),
                            None => break,
                        }
                    }
                    loop {
                        let i = next_seg.fetch_add(1, Relaxed);
                        if i >= segs.len() {
                            break;
                        }
                        let (off, bytes) = segs[i];
                        let t_r0 = Instant::now();
                        if spill.direct {
                            let need = (bytes as usize).next_multiple_of(4096);
                            if abuf.as_ref().map(|b| b.cap() < need).unwrap_or(true) {
                                abuf = Some(crate::extmem::AlignedBuf::new(need.max(SPILL_CAP * 4)));
                            }
                            let sb = abuf.as_mut().unwrap().slice_mut();
                            spill.file.read_exact_at(&mut sb[..need], off).expect("spill read (direct)");
                            t_rd += t_r0.elapsed().as_secs_f64();
                            let view = unsafe { std::slice::from_raw_parts(sb.as_ptr() as *const u32, bytes as usize / 4) };
                            process(view, &mut tl);
                        } else {
                            rbuf.resize(bytes as usize / 4, 0);
                            spill.file.read_exact_at(as_bytes_mut(&mut rbuf), off).expect("spill read");
                            t_rd += t_r0.elapsed().as_secs_f64();
                            process(&rbuf, &mut tl);
                        }
                    }
                    P2_READ_NS.fetch_add((t_rd * 1e9) as u64, Relaxed);
                    P2_TOTAL_NS.fetch_add(t_thread.elapsed().as_nanos() as u64, Relaxed);
                    if bk.direct {
                        return tl; // unaligned tails are written after all O_DIRECT blocks
                    }
                    for (k, v) in tl.iter_mut().enumerate() {
                        bk.flush(k, v, bufcap);
                    }
                    Vec::new()
                })
            })
            .collect();
        for h in hs {
            all_tails.push(h.join().unwrap());
        }
        t_ids = h_ids.join().unwrap();
    });
    if bk.direct {
        bk.dio_len = bk.lens.iter().map(|l| l.load(Relaxed)).collect();
        let bk = &bk;
        std::thread::scope(|s| {
            for tl in all_tails.iter() {
                s.spawn(move || {
                    for (k, v) in tl.iter().enumerate() {
                        if !v.is_empty() {
                            let bytes = (v.len() * 4) as u64;
                            let off = bk.lens[k].fetch_add(bytes, Relaxed);
                            bk.tails[k].write_all_at(as_bytes(v), off).expect("bucket tail write");
                        }
                    }
                });
            }
        });
    }
    drop(all_tails);
    let _ = spill.file.set_len(0);
    let _ = std::fs::remove_file(&spill.path);
    let t_pass2 = t0.elapsed().as_secs_f64();
    let rss_after_p2 = crate::cur_rss();
    drop(cum);
    drop(bitmap);

    // ---------------- pass 3: sort buckets, write targets ----------------
    let t0 = Instant::now();
    let mut offsets: Vec<u64> = vec![0u64; n + 1];
    let off_ptr = SyncPtr(offsets.as_mut_ptr());
    let tfile = File::options().read(true).write(true).create(true).truncate(true).open(cfg.out_dir.join("targets.u32")).map_err(|e| e.to_string())?;
    let raw_entries_total: u64 = bk.lens.iter().map(|l| l.load(Relaxed) / 8).sum();
    let out_map = if cfg.out_mmap && raw_entries_total > 0 {
        tfile.set_len(raw_entries_total * 4).map_err(|e| e.to_string())?;
        Some(unsafe { memmap2::MmapMut::map_mut(&tfile).map_err(|e| e.to_string())? })
    } else {
        None
    };
    let out_ptr = SyncPtr(out_map.as_ref().map(|m| m.as_ptr() as *mut u32).unwrap_or(std::ptr::null_mut()));
    let commit = Mutex::new(Commit { next: 0, base: 0, pending: BTreeMap::new() });
    let pool: Mutex<Vec<Vec<u32>>> = Mutex::new(Vec::new());
    let next_b = AtomicUsize::new(0);
    let wq: Mutex<Vec<(u64, Vec<u32>)>> = Mutex::new(Vec::new());
    let write_out = |base: u64, v: &[u32]| {
        if out_ptr.get().is_null() {
            tfile.write_all_at(as_bytes(v), base * 4).expect("targets write");
        } else {
            unsafe {
                let dst = out_ptr.get().add(base as usize);
                std::ptr::copy_nonoverlapping(v.as_ptr(), dst, v.len());
                // keep resident file pages bounded: unmap the fully written interior pages
                let a = (dst as usize).next_multiple_of(4096);
                let e = (dst as usize + v.len() * 4) & !4095;
                if e > a {
                    libc::madvise(a as *mut _, e - a, libc::MADV_DONTNEED);
                }
            }
        }
    };
    let max_bucket_bytes = bk.lens.iter().map(|l| l.load(Relaxed)).max().unwrap_or(0);
    let (mut p3_read, mut p3_sort, mut p3_write) = (0f64, 0f64, 0f64);
    std::thread::scope(|s| {
        let bk = &bk;
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut raw: Vec<u32> = Vec::new();
                    let mut araw: Option<crate::extmem::AlignedBuf> = None;
                    let mut cnt: Vec<u32> = Vec::new();
                    let (mut tr, mut ts, mut tw) = (0f64, 0f64, 0f64);
                    loop {
                        let b = next_b.fetch_add(1, Relaxed);
                        if b >= nb {
                            break;
                        }
                        let t_a = Instant::now();
                        let lo = b << shift;
                        let hi = ((b + 1) << shift).min(n);
                        let nn = hi - lo;
                        let entries = (bk.lens[b].load(Relaxed) / 8) as usize;
                        let chunks: Vec<Vec<u32>>;
                        let views: Vec<&[u32]> = if bk.direct {
                            let len = bk.lens[b].load(Relaxed) as usize;
                            let dlen = bk.dio_len[b] as usize;
                            let need = len.next_multiple_of(4096) + 4096;
                            if araw.as_ref().map(|x| x.cap() < need).unwrap_or(true) {
                                araw = Some(crate::extmem::AlignedBuf::new(need.max(max_bucket_bytes as usize + 8192)));
                            }
                            let sb = araw.as_mut().unwrap().slice_mut();
                            if dlen > 0 {
                                bk.files[b].read_exact_at(&mut sb[..dlen], 0).expect("bucket read (direct)");
                            }
                            if len > dlen {
                                bk.tails[b].read_exact_at(&mut sb[dlen..len], dlen as u64).expect("bucket tail read");
                            }
                            let _ = bk.files[b].set_len(0);
                            vec![unsafe { std::slice::from_raw_parts(sb.as_ptr() as *const u32, len / 4) }]
                        } else if bk.on_disk {
                            raw.resize(entries * 2, 0);
                            bk.files[b].read_exact_at(as_bytes_mut(&mut raw), 0).expect("bucket read");
                            let _ = bk.files[b].set_len(0); // drop its page cache before writeback
                            vec![&raw[..]]
                        } else {
                            chunks = std::mem::take(&mut *bk.ram[b].lock().unwrap());
                            chunks.iter().map(|c| &c[..]).collect()
                        };
                        let t_b = Instant::now();
                        cnt.clear();
                        cnt.resize(nn + 1, 0);
                        for v in &views {
                            for pr in v.chunks_exact(2) {
                                unsafe { *cnt.get_unchecked_mut(pr[0] as usize - lo + 1) += 1 };
                            }
                        }
                        for i in 0..nn {
                            cnt[i + 1] += cnt[i];
                        }
                        let mut tmp: Vec<u32> = pool.lock().unwrap().pop().unwrap_or_default();
                        tmp.clear();
                        tmp.reserve(entries);
                        unsafe { tmp.set_len(entries) };
                        for v in &views {
                            for pr in v.chunks_exact(2) {
                                unsafe {
                                    let c = cnt.get_unchecked_mut(pr[0] as usize - lo);
                                    *tmp.get_unchecked_mut(*c as usize) = pr[1];
                                    *c += 1;
                                }
                            }
                        }
                        // after the scatter cnt[i] = end of node i
                        let mut w = 0usize;
                        let mut start = 0usize;
                        for i in 0..nn {
                            let end = cnt[i] as usize;
                            tmp[start..end].sort_unstable();
                            let w0 = w;
                            let mut prev = u32::MAX;
                            for j in start..end {
                                let x = tmp[j];
                                if x != prev {
                                    tmp[w] = x;
                                    w += 1;
                                    prev = x;
                                }
                            }
                            unsafe { off_ptr.write(lo + i + 1, (w - w0) as u64) };
                            start = end;
                        }
                        tmp.truncate(w);
                        let t_c = Instant::now();
                        // ordered commit: offsets are assigned in bucket order under a short lock;
                        // the copies themselves go to a shared queue so every worker helps writing
                        {
                            let mut c = commit.lock().unwrap();
                            c.pending.insert(b, tmp);
                            let mut ready: Vec<(u64, Vec<u32>)> = Vec::new();
                            loop {
                                let nx = c.next;
                                match c.pending.remove(&nx) {
                                    Some(v) => {
                                        let base = c.base;
                                        c.base += v.len() as u64;
                                        c.next += 1;
                                        ready.push((base, v));
                                    }
                                    None => break,
                                }
                            }
                            if !ready.is_empty() {
                                wq.lock().unwrap().append(&mut ready);
                            }
                        }
                        let mut done = 0;
                        while done < 2 {
                            let job = wq.lock().unwrap().pop();
                            let Some((base, v)) = job else { break };
                            write_out(base, &v);
                            pool.lock().unwrap().push(v);
                            done += 1;
                        }
                        let t_d = Instant::now();
                        tr += (t_b - t_a).as_secs_f64();
                        ts += (t_c - t_b).as_secs_f64();
                        tw += (t_d - t_c).as_secs_f64();
                    }
                    // final drain: every worker empties the write queue after its last bucket
                    let t_c = Instant::now();
                    loop {
                        let job = wq.lock().unwrap().pop();
                        let Some((base, v)) = job else { break };
                        write_out(base, &v);
                    }
                    tw += t_c.elapsed().as_secs_f64();
                    (tr, ts, tw)
                })
            })
            .collect();
        for h in hs {
            let (a, b, c) = h.join().unwrap();
            p3_read += a;
            p3_sort += b;
            p3_write += c;
        }
    });
    let t_pass3 = t0.elapsed().as_secs_f64();
    let rss_after_p3 = crate::cur_rss();
    for p in &bk.paths {
        let _ = std::fs::remove_file(p);
    }

    // ---------------- finalize ----------------
    let t0 = Instant::now();
    for i in 0..n {
        offsets[i + 1] += offsets[i];
    }
    let entries = offsets[n];
    assert_eq!(entries, commit.lock().unwrap().base);
    if out_map.is_some() {
        drop(out_map);
        tfile.set_len(entries * 4).map_err(|e| e.to_string())?;
    }
    let ofile = File::create(cfg.out_dir.join("offsets.u64")).map_err(|e| e.to_string())?;
    ofile.write_all_at(as_bytes(&offsets), 0).map_err(|e| e.to_string())?;
    let source = match &cfg.input {
        Input::File(p) | Input::GzInProc(p) => p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        Input::Stdin => "stdin".to_string(),
    };
    let meta = json!({
        "format": "glab-csr-v0", "n": n, "m_undirected": entries / 2, "entries": entries,
        "directed": false, "symmetric": true, "sorted": true, "self_loops": false, "source": source,
    });
    std::fs::write(cfg.out_dir.join("meta.json"), serde_json::to_string_pretty(&meta).unwrap()).map_err(|e| e.to_string())?;
    let t_final = t0.elapsed().as_secs_f64();
    let mut t_fsync = 0.0;
    if cfg.fsync {
        let t0 = Instant::now();
        tfile.sync_all().map_err(|e| e.to_string())?;
        ofile.sync_all().map_err(|e| e.to_string())?;
        File::open(&ids_path).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
        t_fsync = t0.elapsed().as_secs_f64();
    }
    let total = t_all.elapsed().as_secs_f64();
    Ok(json!({
        "mode": mode, "threads": threads, "mem_budget_gb": budget as f64 / (1u64 << 30) as f64,
        "n": n, "entries": entries, "m_undirected": entries / 2, "edges_parsed": edges_parsed,
        "text_bytes": text_bytes, "seconds": total,
        "text_gb_per_s": text_bytes as f64 / 1e9 / total, "edges_per_s": edges_parsed as f64 / total,
        "peak_rss_gb": peak_rss() as f64 / (1u64 << 30) as f64,
        "rss_gb_after_pass": [rss_after_p1 as f64 / 1073741824.0, rss_after_p2 as f64 / 1073741824.0, rss_after_p3 as f64 / 1073741824.0],
        "temp_disk_gb": (spill_disk_bytes + if buckets_on_disk { bucket_bytes_total } else { 0 }) as f64 / 1e9,
        "spill_ram_gb": spill_ram_bytes as f64 / 1e9, "spill_disk_gb": spill_disk_bytes as f64 / 1e9,
        "buckets": nb, "buckets_on_disk": buckets_on_disk, "bucket_gb": bucket_bytes_total as f64 / 1e9,
        "max_bucket_mb": max_bucket_bytes as f64 / 1e6, "bucket_buf_u32": bufcap,
        "phase_seconds": {
            "pass1_parse": t_pass1, "rank_directory": t_rank, "pass2_relabel_partition": t_pass2,
            "pass3_sort_dedup_write": t_pass3, "finalize_offsets_meta": t_final, "fsync": t_fsync,
            "ids_writer_concurrent": t_ids,
        },
        "thread_seconds": {
            "p1_read": p1_read_cpu, "p1_parse_mark_spill": p1_parse_cpu,
            "p2_spill_read": P2_READ_NS.load(Relaxed) as f64 / 1e9, "p2_bucket_write": P2_WRITE_NS.load(Relaxed) as f64 / 1e9,
            "p2_total": P2_TOTAL_NS.load(Relaxed) as f64 / 1e9,
            "p3_bucket_read": p3_read, "p3_count_scatter_sort_dedup": p3_sort, "p3_commit_pwrite": p3_write,
            "reader_in_read": t_reader_read, "reader_blocked_on_parsers": t_reader_blocked,
        },
        "fsync": cfg.fsync, "out_mmap": cfg.out_mmap, "direct_in": cfg.direct_in, "direct_tmp": direct_tmp, "prealloc": cfg.prealloc,
    }))
}
