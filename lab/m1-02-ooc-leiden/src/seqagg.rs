//! m1-02: sequential aggregation (issue #16), replacing the member-order `aggregate` when the
//! run is out of core.
//!
//! 1. Dense community ids in increasing label order (exactly as `aggregate`, so level ids and
//!    hashes are the same), `dmap`, and per community: k' (sum of member strengths) and an
//!    upper bound of its entries (sum of member degrees).
//! 2. Communities are cut into contiguous ranges of about equal entry mass: the buckets.
//! 3. One streaming pass over the adjacency in node order (parallel over node ranges). For each
//!    node u the neighbour communities are combined in a per-thread hash table; internal
//!    entries go to an exact per-community self-loop counter; each distinct (c(u), c(v), w) is
//!    appended to the thread's write-combining buffer of bucket(c(u)). Full buffers go to the
//!    bucket's RAM list while the memory accountant allows it, else to the bucket's file
//!    (unlinked temp file, positional writes at an atomically reserved offset): the ingest
//!    bucket machinery of lab/b1-core (pass 2), with a RAM/disk hybrid instead of all-or-nothing.
//! 4. Buckets are processed in parallel, in order of reservation: counting sort by community,
//!    sort + combine each list by target, insert the self-loop, check sum(w) == k'(c), encode
//!    the weighted compressed record; blocks are committed to the output file in bucket order
//!    (ordered commit, as ingest pass 3). Each bucket reserves its memory before it is taken;
//!    the smallest uncommitted bucket always holds its reservation, so the pipeline cannot
//!    deadlock on memory. If a bucket cannot fit even alone, RAM chunks of later buckets are
//!    moved to their files first; if it still does not fit, the run is refused (exit 3).
//!    A community whose entry bound exceeds the bucket target is split over several buckets by
//!    a hash of the target (even over distinct targets, not records: see `part_of`); the parts
//!    are joined and sorted at commit. A broken invariant or an I/O error ends the run at once
//!    (exit 4 / 5, one JSON status line): no thread is left waiting for a bucket.
//! 5. The output (the quotient graph) is mapped read-only from its unlinked file: file-backed
//!    memory, evictable; the next level reads it through the compressed store's random access.
//!
//! Weights: integer and exact (u64 while combining, u32 in the record; asserted). The quotient
//! graph equals `aggregate`'s as a set of (source, target, weight) triples; only the order of
//! each list differs (sorted here, hash-insertion order there). The algorithm does not see the
//! order: every sum it forms over a list is a sum of integers (exact in f64 in any order),
//! candidates are compared by value with a total tie-break order (`tie_less`), so the decision
//! for a node is the same whatever the order of its list. Checked: `--check-agg` (every level
//! equal to `aggregate`) and identical deterministic hashes on lj and orkut.
use super::*;
use crate::acct::{res, sz, try_res, Res};
use crate::store::{encode_record, StoreWriter};
use everygraph_core::MemoryAccountant;
use std::collections::BTreeMap;
use std::fs::File;
use std::os::unix::fs::FileExt;

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Rec {
    cu: u32,
    cv: u32,
    w: u32,
}
const REC: u64 = std::mem::size_of::<Rec>() as u64;

#[derive(Clone, Default)]
pub struct AggStats {
    pub prep_s: f64,
    pub emit_s: f64,
    pub process_s: f64,
    pub finish_s: f64,
    pub records: u64,
    pub ram_bytes: u64,
    pub disk_bytes: u64,
    pub buckets: usize,
    pub bucket_buf_records: usize,
    pub out_bytes: u64,
    pub out_entries: u64,
    pub max_bucket_records: u64,
    pub mem_waits: u64,
    pub split_communities: usize,
    pub demoted_bytes: u64,
}
impl AggStats {
    pub fn json(&self) -> String {
        format!(
            "{{\"prep_s\":{:.4},\"emit_s\":{:.4},\"process_s\":{:.4},\"finish_s\":{:.4},\"records\":{},\"bucket_ram_bytes\":{},\"bucket_disk_bytes\":{},\"buckets\":{},\"bucket_buf_records\":{},\"out_bytes\":{},\"out_entries\":{},\"max_bucket_records\":{},\"mem_waits\":{},\"split_communities\":{},\"demoted_bytes\":{}}}",
            self.prep_s, self.emit_s, self.process_s, self.finish_s, self.records, self.ram_bytes, self.disk_bytes, self.buckets, self.bucket_buf_records, self.out_bytes, self.out_entries, self.max_bucket_records, self.mem_waits, self.split_communities, self.demoted_bytes
        )
    }
}

struct Buckets {
    files: Vec<File>,
    disk_len: Vec<AtomicU64>,
    ram: Vec<Mutex<Vec<(Vec<Rec>, Res)>>>,
    ram_recs: Vec<AtomicU64>,
    ram_bytes: AtomicU64,
    disk_bytes: AtomicU64,
    demoted: AtomicU64,
    allow_ram: bool,
}
impl Buckets {
    /// Tests: write a buffer to the bucket's file whatever the budget.
    #[cfg(test)]
    fn allow_ram_off_flush(&self, b: usize, v: &mut Vec<Rec>) {
        let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * REC as usize) };
        let off = self.disk_len[b].fetch_add(bytes.len() as u64, Relaxed);
        self.files[b].write_all_at(bytes, off).unwrap();
        v.clear();
    }
    fn flush(&self, b: usize, v: &mut Vec<Rec>, cap: usize) {
        if v.is_empty() {
            return;
        }
        if self.allow_ram {
            if let Some(r) = try_res("aggregation: bucket chunks kept in RAM", sz::<Rec>(v.capacity())) {
                let full = std::mem::replace(v, Vec::with_capacity(cap));
                self.ram_recs[b].fetch_add(full.len() as u64, Relaxed);
                self.ram_bytes.fetch_add(sz::<Rec>(full.len()), Relaxed);
                self.ram[b].lock().unwrap().push((full, r));
                return;
            }
        }
        let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * REC as usize) };
        let off = self.disk_len[b].fetch_add(bytes.len() as u64, Relaxed);
        self.files[b].write_all_at(bytes, off).unwrap_or_else(|e| io_failed("aggregation: bucket write", e));
        self.disk_bytes.fetch_add(bytes.len() as u64, Relaxed);
        v.clear();
    }
}

/// Moves RAM chunks of buckets `from..` (latest first) to their files until `need` bytes are
/// available. Returns whether anything was moved.
fn demote(bk: &Buckets, from: usize, need: u64) -> bool {
    let mut moved = false;
    for b in (from..bk.ram.len()).rev() {
        if crate::acct::acct().available() >= need {
            break;
        }
        let chunks = std::mem::take(&mut *bk.ram[b].lock().unwrap());
        for (v, r) in chunks {
            let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * REC as usize) };
            let off = bk.disk_len[b].fetch_add(bytes.len() as u64, Relaxed);
            debug_assert_eq!(off % REC, 0);
            bk.files[b].write_all_at(bytes, off).unwrap_or_else(|e| io_failed("aggregation: bucket write", e));
            bk.disk_bytes.fetch_add(bytes.len() as u64, Relaxed);
            bk.ram_recs[b].fetch_sub(v.len() as u64, Relaxed);
            bk.demoted.fetch_add(bytes.len() as u64, Relaxed);
            drop(v);
            drop(r);
            moved = true;
        }
    }
    moved
}

/// Part of a split community that target `cv` goes to (a multiplicative hash). It spreads the
/// distinct targets evenly, not the records: a giant community whose members mostly link to one
/// other community puts most of its records in one part. The result stays correct (`need_of`
/// counts the real records), but such a part can make the run refuse (exit 3) at a budget where
/// an even spread would have fit. Splitting by measured record counts is left to #60.
#[inline(always)]
fn part_of(cv: u32, parts: usize) -> usize {
    ((cv.wrapping_mul(0x9E37_79B1) as u64 * parts as u64) >> 32) as usize
}

struct EmitTls {
    tab: Tab,
    bufs: Vec<Vec<Rec>>,
    records: u64,
}

/// Output of one bucket, waiting for its turn in the file.
enum Out {
    /// encoded records of communities lo..hi
    Block { lo: usize, hi: usize, bytes: Vec<u8>, rel: Vec<u32>, deg: Vec<u32>, _res: Res },
    /// part `part` of `parts` of split community c: combined list of one target range
    Part { c: usize, part: u32, parts: u32, tl: Vec<u32>, wl: Vec<u32>, sum: u64, res: Res },
}

/// Sorted (target << 32 | weight) pairs -> combined lists; returns the sum of weights.
fn combine(s: &[u64], tl: &mut Vec<u32>, wl: &mut Vec<u32>) -> u64 {
    let mut sum = 0u64;
    let mut j = 0;
    while j < s.len() {
        let cv = (s[j] >> 32) as u32;
        let mut w = 0u64;
        while j < s.len() && (s[j] >> 32) as u32 == cv {
            w += s[j] & 0xFFFF_FFFF;
            j += 1;
        }
        tl.push(cv);
        wl.push(to_u32(w, "quotient weight"));
        sum += w;
    }
    sum
}

/// Inserts the self-loop of c (if any) at its sorted place; returns the row sum.
fn finish_list(c: usize, tl: &mut Vec<u32>, wl: &mut Vec<u32>, sum: u64, selfw: u64) -> u64 {
    if selfw > 0 {
        let p = tl.partition_point(|&x| x < c as u32);
        debug_assert!(tl.get(p) != Some(&(c as u32)));
        tl.insert(p, c as u32);
        wl.insert(p, to_u32(selfw, "self-loop weight"));
    }
    sum + selfw
}

/// Ends the run at once on a broken invariant (exit 4, one JSON status line): no thread waits
/// for a bucket that will never be committed (review of PR #143, S1).
pub fn invariant_failed(msg: String) -> ! {
    println!("{}", serde_json::json!({"status": "failed", "error": format!("invariant violated: {msg}")}));
    eprintln!("invariant violated: {msg}");
    std::process::exit(4)
}
/// Ends the run on an I/O error of the spill path (exit 5, one JSON status line), e.g. ENOSPC:
/// a refusal with the reason instead of a panic. There is no disk budget (not implemented).
pub fn io_failed(what: &str, e: std::io::Error) -> ! {
    println!("{}", serde_json::json!({"status": "refused", "error": format!("{what}: {e}"), "purpose": what}));
    eprintln!("{what}: {e}");
    std::process::exit(5)
}
/// Test hook: M102_INJECT_ROW_FAULT=c makes community c's row-sum check fail.
fn injected_fault() -> Option<usize> {
    static F: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("M102_INJECT_ROW_FAULT").ok().and_then(|s| s.parse().ok()))
}
fn to_u32(w: u64, what: &str) -> u32 {
    u32::try_from(w).unwrap_or_else(|_| invariant_failed(format!("{what} {w} exceeds u32")))
}

/// Invariant: the row sum of a community is its strength k' (exact integers). Aborts otherwise.
fn check_row_sum(c: usize, sum: u64, kc: f64) {
    if sum as f64 != kc || injected_fault() == Some(c) {
        invariant_failed(format!("k_equals_row_sum: community {c}: row sum {sum}, k' {kc}"));
    }
}

fn raise_nofile() {
    unsafe {
        let mut r: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) == 0 && r.rlim_cur < r.rlim_max {
            r.rlim_cur = r.rlim_max;
            libc::setrlimit(libc::RLIMIT_NOFILE, &r);
        }
    }
}

static SPILL_SEQ: AtomicUsize = AtomicUsize::new(0);

/// Aggregates g by labels `lab` (labels in [0, n)). Returns (quotient graph, its index
/// reservation, k', dmap, stats). The quotient graph is a weighted compressed store mapped from
/// an unlinked file under `spill`.
pub fn aggregate_seq(g: &Graph, k: &[f64], lab: &[AtomicU32], pool: &rayon::ThreadPool, spill: &str, keep: Option<&str>) -> SeqOut {
    let t0 = Instant::now();
    let n = g.n;
    let nt = pool.current_num_threads();
    let mut st = AggStats::default();
    // ---- 1. dense ids by increasing label, dmap, k', entry upper bounds
    let r_cnt = res("aggregation: label counts / new ids", sz::<u32>(n));
    let mut cnt_v = vec![0u32; n];
    {
        let cnt = as_atomic_u32(&mut cnt_v);
        par_for(pool, n, 8192, |_, r| {
            for u in r {
                cnt[lab[u].load(Relaxed) as usize].fetch_add(1, Relaxed);
            }
        });
    }
    let mut c_new = 0usize;
    for x in cnt_v.iter_mut() {
        if *x > 0 {
            *x = c_new as u32;
            c_new += 1;
        } else {
            *x = NONE;
        }
    }
    let r_dmap = res("aggregation: dmap", sz::<u32>(n));
    let mut dmap = vec![0u32; n];
    {
        let dp = SyncPtr(dmap.as_mut_ptr());
        let newid = &cnt_v;
        par_for(pool, n, 8192, |_, r| {
            for u in r {
                unsafe { dp.write(u, newid[lab[u].load(Relaxed) as usize]) };
            }
        });
    }
    drop(cnt_v);
    drop(r_cnt);
    let r_comm = res("aggregation: per-community k', self-loops, bounds, bucket map", (sz::<u64>(c_new) * 3) + sz::<u32>(c_new));
    let mut knew_bits = vec![0u64; c_new];
    let mut selfw_v = vec![0u64; c_new];
    let mut ub_v = vec![0u64; c_new];
    {
        let kn = as_atomic_u64(&mut knew_bits);
        let ub = as_atomic_u64(&mut ub_v);
        let dm = &dmap;
        par_for(pool, n, 8192, |_, r| {
            for u in r {
                let c = dm[u] as usize;
                fadd(&kn[c], k[u]); // integer strengths: exact in any order
                ub[c].fetch_add(g.deg(u) as u64, Relaxed);
            }
        });
    }
    // ---- 2. buckets: contiguous community ranges of about equal entry mass
    let total_ub: u64 = ub_v.iter().sum();
    let r_writer = res("aggregation: output offsets and degrees", StoreWriter::ram_bytes(c_new));
    let avail = crate::acct::acct().available();
    let mut target = (avail / 4 / nt as u64 / 32).clamp(1 << 14, 1 << 26);
    // at most 2048 buckets, and no more than the write-combining buffers (64 records minimum
    // per thread and bucket) can have with an eighth of what is available
    let nb_max = ((avail / 8) / (nt as u64 * 64 * REC)).clamp(1, 2048);
    if total_ub / target > nb_max {
        target = total_ub / nb_max + 1;
    }
    if nt > 1 && total_ub / target < (4 * nt) as u64 {
        target = (total_ub / (4 * nt) as u64).max(1 << 14);
    }
    // Buckets in community order. A community whose entry bound exceeds the target is split
    // over p consecutive buckets by a hash of the target (even over distinct targets, not over
    // records: see part_of);
    // its disjoint partial lists are joined and sorted at commit time. bfirst[c] = first bucket of c, and
    // bfirst[c + 1] - bfirst[c] = p for a split community (0 or 1 otherwise).
    let mut bkt: Vec<(usize, usize, u32, u32)> = Vec::new(); // (lo, hi, part, parts)
    let mut bfirst = vec![0u32; c_new + 1];
    {
        let mut acc = 0u64;
        let mut open = false;
        for c in 0..c_new {
            if ub_v[c] > target && c_new > 1 {
                if open {
                    bkt.last_mut().unwrap().1 = c;
                    open = false;
                }
                let p = ub_v[c].div_ceil(target).min(c_new as u64).max(2) as u32;
                bfirst[c] = bkt.len() as u32;
                for j in 0..p {
                    bkt.push((c, c + 1, j, p));
                }
                acc = 0;
                continue;
            }
            if !open {
                bkt.push((c, c_new, 0, 1));
                open = true;
            }
            bfirst[c] = (bkt.len() - 1) as u32;
            acc += ub_v[c];
            if acc >= target {
                bkt.last_mut().unwrap().1 = c + 1;
                open = false;
                acc = 0;
            }
        }
        if open {
            bkt.last_mut().unwrap().1 = c_new;
        }
        bfirst[c_new] = bkt.len() as u32;
    }
    let nb = bkt.len();
    let max_bucket_ub = bkt.iter().map(|&(lo, hi, _, p)| ub_v[lo..hi].iter().sum::<u64>() / p as u64).max().unwrap_or(0);
    st.split_communities = bkt.iter().filter(|b| b.2 == 0 && b.3 > 1).count();
    drop(ub_v);
    let bmap = bfirst;
    raise_nofile();
    std::fs::create_dir_all(spill).unwrap_or_else(|e| io_failed("aggregation: spill directory", e));
    let seq = SPILL_SEQ.fetch_add(1, Relaxed);
    let mut files = Vec::with_capacity(nb);
    for b in 0..nb {
        let p = std::path::Path::new(spill).join(format!("bucket.{}.{seq}.{b:05}", std::process::id()));
        let f = File::options().read(true).write(true).create(true).truncate(true).open(&p).unwrap_or_else(|e| io_failed("aggregation: bucket file", e));
        std::fs::remove_file(&p).unwrap_or_else(|e| io_failed("aggregation: unlink bucket file", e));
        files.push(f);
    }
    let bk = Buckets {
        files,
        disk_len: (0..nb).map(|_| AtomicU64::new(0)).collect(),
        ram: (0..nb).map(|_| Mutex::new(Vec::new())).collect(),
        ram_recs: (0..nb).map(|_| AtomicU64::new(0)).collect(),
        ram_bytes: AtomicU64::new(0),
        disk_bytes: AtomicU64::new(0),
        demoted: AtomicU64::new(0),
        allow_ram: true,
    };
    // headroom for step 4 (sort buffers of nt buckets), so that RAM chunks cannot starve it
    let avail = crate::acct::acct().available();
    let headroom = (nt as u64 * max_bucket_ub * 24).min(avail / 2);
    let r_head = res("aggregation: headroom for bucket sorting", headroom);
    let avail = crate::acct::acct().available();
    let cap = ((avail / 10) / (nt as u64 * nb as u64 * REC)).clamp(64, 8192) as usize;
    let r_bufs = res("aggregation: write-combining buffers", nt as u64 * nb as u64 * cap as u64 * REC);
    st.buckets = nb;
    st.bucket_buf_records = cap;
    st.prep_s = t0.elapsed().as_secs_f64();
    // ---- 3. streaming pass
    let t1 = Instant::now();
    let selfw = as_atomic_u64(&mut selfw_v);
    let tls: Vec<Mutex<EmitTls>> = (0..nt).map(|_| Mutex::new(EmitTls { tab: Tab::new(), bufs: (0..nb).map(|_| Vec::new()).collect(), records: 0 })).collect();
    {
        let dm = &dmap;
        let bm = &bmap;
        let bk = &bk;
        par_for(pool, n, 4096, |tid, r| {
            let mut guard = tls[tid].lock().unwrap();
            let t = &mut *guard;
            for u in r {
                let deg = g.deg(u);
                if deg == 0 {
                    continue;
                }
                let cu = dm[u];
                let _gg = giant_gate(deg);
                let mut own = 0f64;
                t.tab.begin(deg);
                g.for_edges(u, |v, w| {
                    let cv = unsafe { *dm.get_unchecked(v as usize) };
                    if cv == cu {
                        own += w
                    } else {
                        t.tab.add(cv, w)
                    }
                });
                if own > 0.0 {
                    selfw[cu as usize].fetch_add(own as u64, Relaxed);
                }
                let b0 = bm[cu as usize] as usize;
                let parts = bm[cu as usize + 1] as usize - b0; // > 1: a split community
                let EmitTls { tab, bufs, records } = t;
                for &s in &tab.used {
                    let w = tab.vals[s as usize];
                    let cv = tab.keys[s as usize];
                    debug_assert!(w >= 1.0 && w <= u32::MAX as f64 && w.fract() == 0.0);
                    let b = if parts > 1 { b0 + part_of(cv, parts) } else { b0 };
                    let buf = &mut bufs[b];
                    if buf.capacity() == 0 {
                        buf.reserve_exact(cap);
                    }
                    buf.push(Rec { cu, cv, w: to_u32(w as u64, "combined per-node weight") });
                    if buf.len() >= cap {
                        bk.flush(b, buf, cap);
                    }
                }
                *records += tab.used.len() as u64;
                tab.clear();
            }
        });
        // tails
        let tl: Vec<EmitTls> = tls.into_iter().map(|m| m.into_inner().unwrap()).collect();
        let tl = Mutex::new(tl);
        par_for(pool, nt, 1, |_, r| {
            for i in r {
                let mut bufs = std::mem::take(&mut tl.lock().unwrap()[i].bufs);
                for (b, v) in bufs.iter_mut().enumerate() {
                    bk.flush(b, v, 0);
                }
            }
        });
        st.records = tl.into_inner().unwrap().iter().map(|t| t.records).sum();
    }
    drop(r_bufs);
    drop(r_head);
    unsafe { libc::malloc_trim(0) };
    st.ram_bytes = bk.ram_bytes.load(Relaxed);
    st.disk_bytes = bk.disk_bytes.load(Relaxed);
    st.emit_s = t1.elapsed().as_secs_f64();
    // ---- 4. per-bucket sort / combine / encode, ordered commit
    let t2 = Instant::now();
    // the quotient graph: an unlinked spill file, or (--out-hier out of core) a store directory
    // kept on disk for the map, never held in memory
    let keep_dir = keep.map(|d| std::path::Path::new(d).join(format!("quot_{seq}")));
    let path = match &keep_dir {
        Some(d) => {
            std::fs::create_dir_all(d).unwrap_or_else(|e| io_failed("aggregation: quotient directory", e));
            d.join("adj.bin")
        }
        None => std::path::Path::new(spill).join(format!("quotient.{}.{seq}.adj", std::process::id())),
    };
    let writer = Mutex::new(StoreWriter::create(&path, c_new, true).unwrap_or_else(|e| io_failed("aggregation: quotient file", e)));
    if keep_dir.is_none() {
        writer.lock().unwrap().unlink_now().unwrap_or_else(|e| io_failed("aggregation: unlink quotient file", e));
    }
    let next_take = Mutex::new(0usize);
    let in_flight = AtomicUsize::new(0);
    // (next bucket to commit, waiting outputs, assembler of a split community: c, targets,
    // weights, sum, reservations)
    type Asm = (usize, Vec<u32>, Vec<u32>, u64, Vec<Res>);
    let commit: Mutex<(usize, BTreeMap<usize, Out>, Asm)> = Mutex::new((0, BTreeMap::new(), (0, Vec::new(), Vec::new(), 0, Vec::new())));
    let wfile = writer.lock().unwrap().file_handle();
    let waits = AtomicU64::new(0);
    let max_recs = AtomicU64::new(0);
    let knew_a = as_atomic_u64(&mut knew_bits);
    let need_of = |b: usize| -> (u64, u64) {
        let nn = (bkt[b].1 - bkt[b].0) as u64;
        let disk = bk.disk_len[b].load(Relaxed);
        let recs = bk.ram_recs[b].load(Relaxed) + disk / REC;
        // read buffer + pairs + counts and scatter positions + output (records ~5 B per entry +
        // offsets/degrees; measured 2.7 B per record on Friendster's level 0; growth beyond it is
        // reserved after encoding)
        (disk + recs * 8 + 2 * (nn + 1) * 4 + recs * 5 + nn * 16 + 4096, recs)
    };
    pool.broadcast(|_| {
        let mut tmp: Vec<u32> = Vec::new();
        let mut tl: Vec<u32> = Vec::new();
        let mut wl: Vec<u32> = Vec::new();
        // the per-thread list buffers, reserved as they grow
        let mut r_tl = Res::empty("aggregation: per-thread list buffers");
        loop {
            // take the next bucket together with its reservation
            let (b, mut r) = loop {
                let mut nx = next_take.lock().unwrap();
                let b = *nx;
                if b >= nb {
                    return;
                }
                let (need, _) = need_of(b);
                if let Some(r) = try_res("aggregation: bucket sort and output", need) {
                    *nx += 1;
                    in_flight.fetch_add(1, SeqCst);
                    break (b, r);
                }
                let idle = in_flight.load(SeqCst) == 0;
                if idle && demote(&bk, b + 1, need) {
                    // RAM chunks of later buckets went to their files: try again
                    continue;
                }
                if idle {
                    // nothing else holds memory of this phase: this bucket alone does not fit
                    *nx += 1;
                    in_flight.fetch_add(1, SeqCst);
                    break (b, res("aggregation: bucket sort and output (alone)", need));
                }
                drop(nx);
                waits.fetch_add(1, Relaxed);
                std::thread::sleep(std::time::Duration::from_micros(500));
            };
            let (lo, hi, part, parts) = bkt[b];
            let nn = hi - lo;
            let (_, recs) = need_of(b);
            max_recs.fetch_max(recs, Relaxed);
            // gather the records: RAM chunks and the file part
            let chunks = std::mem::take(&mut *bk.ram[b].lock().unwrap());
            let dlen = bk.disk_len[b].load(Relaxed) as usize;
            let mut disk: Vec<Rec> = vec![Rec::default(); dlen / REC as usize];
            if dlen > 0 {
                let bytes = unsafe { std::slice::from_raw_parts_mut(disk.as_mut_ptr() as *mut u8, dlen) };
                debug_assert_eq!(dlen as u64 % REC, 0);
                bk.files[b].read_exact_at(bytes, 0).unwrap_or_else(|e| io_failed("aggregation: bucket read", e));
                let _ = bk.files[b].set_len(0);
            }
            // counting sort by community
            let mut cnt = vec![0u32; nn + 1];
            let mut count = |v: &[Rec]| {
                for x in v {
                    cnt[x.cu as usize - lo + 1] += 1;
                }
            };
            for (c, _) in &chunks {
                count(c);
            }
            count(&disk);
            for i in 0..nn {
                cnt[i + 1] += cnt[i];
            }
            assert_eq!(cnt[nn] as u64, recs);
            let mut pairs: Vec<u64> = uninit_vec(recs as usize);
            {
                let mut pos = cnt.clone();
                let mut scatter = |v: &[Rec]| {
                    for x in v {
                        let p = &mut pos[x.cu as usize - lo];
                        unsafe { *pairs.get_unchecked_mut(*p as usize) = ((x.cv as u64) << 32) | x.w as u64 };
                        *p += 1;
                    }
                };
                for (c, _) in &chunks {
                    scatter(c);
                }
                scatter(&disk);
            }
            drop(chunks);
            drop(disk);
            let out = if parts > 1 {
                // one part of a split community: its combined (target, weight) list for this
                // target range; joined with the other parts at commit time
                let s = &mut pairs[..];
                s.sort_unstable();
                let (mut tl, mut wl) = (Vec::new(), Vec::new());
                let sum = combine(s, &mut tl, &mut wl);
                drop(pairs);
                r.fit((tl.capacity() * 4 + wl.capacity() * 4) as u64);
                Out::Part { c: lo, part, parts, tl, wl, sum, res: r }
            } else {
                // combine and encode
                let mut out: Vec<u8> = Vec::with_capacity(recs as usize * 5 + nn * 2);
                let mut rel: Vec<u32> = Vec::with_capacity(nn);
                let mut degs: Vec<u32> = Vec::with_capacity(nn);
                for i in 0..nn {
                    let c = lo + i;
                    let s = &mut pairs[cnt[i] as usize..cnt[i + 1] as usize];
                    s.sort_unstable();
                    tl.clear();
                    wl.clear();
                    let sum = combine(s, &mut tl, &mut wl);
                    let sum = finish_list(c, &mut tl, &mut wl, sum, selfw[c].load(Relaxed));
                    check_row_sum(c, sum, f64::from_bits(knew_a[c].load(Relaxed)));
                    rel.push(out.len() as u32);
                    degs.push(tl.len() as u32);
                    encode_record(c, &tl, Some(&wl), &mut out, &mut tmp);
                    let held = ((tl.capacity() + wl.capacity() + tmp.capacity()) * 4) as u64;
                    if held > r_tl.bytes() {
                        r_tl.fit(held);
                    }
                }
                drop(pairs);
                assert!(out.len() < u32::MAX as usize, "bucket output larger than 4 GiB");
                // the reservation follows the output: shrinks, or grows if the estimate was short
                r.fit((out.capacity() + rel.capacity() * 4 + degs.capacity() * 4) as u64);
                Out::Block { lo, hi, bytes: out, rel, deg: degs, _res: r }
            };
            drop(cnt);
            // ordered commit
            let mut ready: Vec<(u64, Vec<u8>, Vec<Res>)> = Vec::new();
            {
                let mut cm = commit.lock().unwrap();
                cm.1.insert(b, out);
                loop {
                    let nx = cm.0;
                    let Some(o) = cm.1.remove(&nx) else { break };
                    cm.0 += 1;
                    match o {
                        Out::Block { lo, hi, bytes, rel, deg, _res } => {
                            let off = writer.lock().unwrap().place(lo, hi, bytes.len(), &rel, &deg);
                            ready.push((off, bytes, vec![_res]));
                        }
                        Out::Part { c, part, parts, tl, wl, sum, res } => {
                            let asm = &mut cm.2;
                            if part == 0 {
                                asm.0 = c;
                                asm.1.clear();
                                asm.2.clear();
                                asm.3 = 0;
                            }
                            assert_eq!(asm.0, c);
                            asm.1.extend_from_slice(&tl);
                            asm.2.extend_from_slice(&wl);
                            asm.3 += sum;
                            asm.4.push(res);
                            if part + 1 == parts {
                                let (mut t, mut w) = (std::mem::take(&mut asm.1), std::mem::take(&mut asm.2));
                                // sort keys (8 B) and the encoded record (at most ~9 B per entry)
                                let mut r_asm = crate::acct::res("aggregation: split community assembly", (t.len() * 17 + 64) as u64);
                                // parts hold disjoint target sets: one sort makes the list
                                let mut z: Vec<u64> = t.iter().zip(&w).map(|(&a, &b)| ((a as u64) << 32) | b as u64).collect();
                                z.sort_unstable();
                                for (i, &x) in z.iter().enumerate() {
                                    t[i] = (x >> 32) as u32;
                                    w[i] = x as u32;
                                }
                                drop(z);
                                let sum = finish_list(c, &mut t, &mut w, asm.3, selfw[c].load(Relaxed));
                                check_row_sum(c, sum, f64::from_bits(knew_a[c].load(Relaxed)));
                                let mut bytes = Vec::new();
                                encode_record(c, &t, Some(&w), &mut bytes, &mut tmp);
                                r_asm.fit(bytes.capacity() as u64);
                                asm.4.push(r_asm);
                                let off = writer.lock().unwrap().place(c, c + 1, bytes.len(), &[0], &[t.len() as u32]);
                                ready.push((off, bytes, std::mem::take(&mut asm.4)));
                            }
                        }
                    }
                }
            }
            for (off, bytes, _) in &ready {
                wfile.write_all_at(bytes, *off).unwrap_or_else(|e| io_failed("aggregation: quotient write", e));
            }
            drop(ready);
            in_flight.fetch_sub(1, SeqCst);
        }
    });
    st.mem_waits = waits.load(Relaxed);
    st.demoted_bytes = bk.demoted.load(Relaxed);
    st.max_bucket_records = max_recs.load(Relaxed);
    st.process_s = t2.elapsed().as_secs_f64();
    drop(bk);
    let t3 = Instant::now();
    let w = writer.into_inner().unwrap();
    assert_eq!(commit.lock().unwrap().0, nb);
    let comp = match &keep_dir {
        Some(d) => w.finish_dir(d, false).unwrap_or_else(|e| io_failed("aggregation: finish quotient store", e)),
        None => w.finish(false).unwrap_or_else(|e| io_failed("aggregation: finish quotient", e)),
    };
    drop(r_writer);
    let r_index = res("quotient graph: offsets index", comp.index_ram_bytes());
    let r_file = crate::acct::file_backed("quotient graph: mapped records", comp.data_len as u64);
    st.out_bytes = comp.data_len as u64;
    st.out_entries = comp.entries as u64;
    drop(selfw_v);
    drop(bmap);
    drop(r_comm);
    let r_knew = res("level state: k' of the next level", sz::<f64>(c_new));
    let knew: Vec<f64> = knew_bits.iter().map(|&b| f64::from_bits(b)).collect();
    drop(knew_bits);
    unsafe { libc::malloc_trim(0) };
    st.finish_s = t3.elapsed().as_secs_f64();
    SeqOut { g: Graph::from_comp(comp), knew, dmap, res: vec![r_index, r_file, r_knew, r_dmap], st }
}

pub struct SeqOut {
    pub g: Graph,
    pub knew: Vec<f64>,
    pub dmap: Vec<u32>,
    /// reservations of g's index, knew and dmap: drop them with those
    pub res: Vec<Res>,
    pub st: AggStats,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Demotion: RAM chunks of later buckets go to their files (appended after what is there),
    /// their reservations are released, and the records read back unchanged.
    #[test]
    fn demote_moves_ram_chunks_to_files_and_releases_them() {
        let _l = crate::acct::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::acct::set_budget_for_tests(u64::MAX);
        let dir = std::env::temp_dir().join(format!("m102-demote-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let nb = 3;
        let files: Vec<File> = (0..nb)
            .map(|b| {
                let p = dir.join(format!("b{b}"));
                let f = File::options().read(true).write(true).create(true).truncate(true).open(&p).unwrap();
                std::fs::remove_file(&p).unwrap();
                f
            })
            .collect();
        let bk = Buckets {
            files,
            disk_len: (0..nb).map(|_| AtomicU64::new(0)).collect(),
            ram: (0..nb).map(|_| Mutex::new(Vec::new())).collect(),
            ram_recs: (0..nb).map(|_| AtomicU64::new(0)).collect(),
            ram_bytes: AtomicU64::new(0),
            disk_bytes: AtomicU64::new(0),
            demoted: AtomicU64::new(0),
            allow_ram: true,
        };
        let rec = |b: u32, i: u32| Rec { cu: b, cv: i, w: i + 1 };
        // bucket 2: one record on disk first, then two RAM chunks
        let mut v = vec![rec(2, 0)];
        bk.allow_ram_off_flush(2, &mut v);
        for (b, n) in [(1u32, 100u32), (2, 50), (2, 70)] {
            let mut v: Vec<Rec> = (0..n).map(|i| rec(b, i + 1)).collect();
            bk.flush(b as usize, &mut v, 0);
        }
        let used = crate::acct::acct().used();
        assert!(used >= sz::<Rec>(220));
        assert!(demote(&bk, 1, u64::MAX));
        assert_eq!(crate::acct::acct().used(), used - sz::<Rec>(220), "chunk reservations not released");
        assert_eq!(bk.ram_recs[1].load(Relaxed) + bk.ram_recs[2].load(Relaxed), 0);
        assert_eq!(bk.demoted.load(Relaxed), 220 * REC);
        let read = |b: usize| -> Vec<(u32, u32, u32)> {
            let len = bk.disk_len[b].load(Relaxed) as usize;
            let mut buf = vec![0u8; len];
            bk.files[b].read_exact_at(&mut buf, 0).unwrap();
            buf.chunks_exact(12).map(|c| (u32::from_le_bytes(c[0..4].try_into().unwrap()), u32::from_le_bytes(c[4..8].try_into().unwrap()), u32::from_le_bytes(c[8..12].try_into().unwrap()))).collect()
        };
        let b1 = read(1);
        assert_eq!(b1.len(), 100);
        assert!(b1.iter().enumerate().all(|(i, &(cu, cv, w))| cu == 1 && cv == i as u32 + 1 && w == cv + 1));
        let mut b2 = read(2);
        assert_eq!(b2.len(), 121);
        assert_eq!(b2[0], (2, 0, 1), "the record already on disk must stay first");
        b2.sort_unstable();
        let mut want: Vec<(u32, u32, u32)> = vec![(2, 0, 1)];
        want.extend((1..=50).map(|i| (2, i, i + 1)));
        want.extend((1..=70).map(|i| (2, i, i + 1)));
        want.sort_unstable();
        assert_eq!(b2, want);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
