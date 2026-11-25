//! The proposed default storage (DESIGN D4), as a prototype:
//!
//!   adj.bin    one byte-aligned record per node, in node order, grouped in node-aligned blocks of
//!              ~64K entries (a node never straddles a block; the block is the unit of parallel
//!              work, of I/O and of sequential decode). Record = [deg: LEB128] then, if deg > 0,
//!              Stream VByte of deg values: zigzag(first - v), then gaps - 1. 64 zero bytes of
//!              padding at the end (SIMD over-read). Byte-identical to lab C's `svb` codec.
//!   boff.ef    Elias-Fano of the n+1 record byte offsets (random access to one list).
//!   blocks.u32 first node of each block, then n.
//!   meta.json
//!
//! Degrees are not stored separately: deg(v) reads the record header (one Elias-Fano lookup and
//! one data byte or two).
//!
//! m1-02 additions (marked "m1-02"): a *weighted* variant for quotient graphs, whose record is
//! the unweighted record followed by a second Stream VByte stream of deg values (weight - 1;
//! integer weights >= 1, which is what aggregating an unweighted graph produces); a streaming
//! writer (`StoreWriter`) that appends encoded blocks to a file in node order and keeps only the
//! n + 1 byte offsets in memory; `max_deg` in the metadata.
use crate::adj::*;
use crate::ef::EliasFano;
use crate::svb;
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

/// Decoder, set once at start-up:
///   0 "lab"   the lab C path: shuffle, then a scalar prefix-sum pass over the buffer
///   1 "fused" shuffle + SIMD prefix sum in one pass, scalar loop for the last 1-3 values
///   2 "pad"   as fused, last group decoded with the shuffle too (no data-dependent branch);
///             used wherever the output buffer has 3 values of slack (default)
pub static DECODER: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);
pub fn decoder_name() -> &'static str {
    ["lab", "fused", "pad"][DECODER.load(Relaxed) as usize]
}
/// Sequential access (`for_range`): how many entries to decode into the per-thread buffer before
/// handing the lists to the kernel. 1 = list by list (decode and kernel body interleaved);
/// usize::MAX = the whole work chunk (one ~64K-entry block). Default 8192 entries (32 KiB, L1-sized).
pub static BATCH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(8192);
/// Prefix scans (`for_range_prefix`, used by Afforest's sampling rounds): decode the prefixes of
/// this many nodes before running the kernel body over them. 0 = node by node.
pub static PREFIX_BATCH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Build, at open, an Elias-Fano index of the *entry* offsets (cumulative degrees) and answer
/// deg(v) from it instead of reading the record header. Not part of the D4 proposal; measured as
/// an option because edge-indexed property columns need these offsets anyway.
pub static DEG_INDEX: AtomicBool = AtomicBool::new(false);

#[inline(always)]
pub fn put_varint(mut x: u64, out: &mut Vec<u8>) {
    while x >= 0x80 {
        out.push((x as u8) | 0x80);
        x >>= 7;
    }
    out.push(x as u8);
}
/// # Safety: `d + *pos` must point at a valid LEB128 value inside the padded data.
#[inline(always)]
unsafe fn get_varint(d: *const u8, pos: &mut usize) -> usize {
    let b = *d.add(*pos);
    *pos += 1;
    if b < 0x80 {
        return b as usize;
    }
    let mut x = (b & 0x7f) as usize;
    let mut sh = 7;
    loop {
        let b = *d.add(*pos);
        *pos += 1;
        x |= ((b & 0x7f) as usize) << sh;
        if b < 0x80 {
            return x;
        }
        sh += 7;
    }
}
#[inline(always)]
fn zz(x: i64) -> u64 {
    ((x << 1) ^ (x >> 63)) as u64
}
#[inline(always)]
fn unzz(x: u64) -> i64 {
    ((x >> 1) as i64) ^ -((x & 1) as i64)
}

/// Decodes the first k of `deg` neighbors of v from the stream at `d` into out[..k].
/// Returns the stream length in bytes if k == deg. `SLACK`: out has room for k + 3 values.
///
/// # Safety: valid padded stream, out has room for k (+ 3 if SLACK) values.
#[inline(always)]
unsafe fn decode_list_g<const SLACK: bool>(d: *const u8, v: usize, deg: usize, k: usize, out: *mut u32) -> usize {
    debug_assert!(k >= 1 && k <= deg);
    let dec = DECODER.load(Relaxed);
    if dec >= 1 {
        let v0 = svb::peek_first(d, deg);
        let x0 = (v as i64 + unzz(v0 as u64)) as u32;
        let base = x0.wrapping_sub(v0).wrapping_sub(1);
        if SLACK && dec == 2 {
            svb::decode_delta_pad(d, deg, k, base, out)
        } else {
            svb::decode_delta(d, deg, k, base, out)
        }
    } else {
        let used = svb::decode(d, deg, k, out);
        let b = std::slice::from_raw_parts_mut(out, k);
        let mut x = (v as i64 + unzz(b[0] as u64)) as u32;
        b[0] = x;
        for y in b[1..].iter_mut() {
            x = x + 1 + *y;
            *y = x;
        }
        used
    }
}
/// Into a scratch buffer (3 values of slack).
#[inline(always)]
unsafe fn decode_list(d: *const u8, v: usize, deg: usize, k: usize, out: *mut u32) -> usize {
    decode_list_g::<true>(d, v, deg, k, out)
}
/// Into an exactly sized destination.
#[inline(always)]
unsafe fn decode_list_exact(d: *const u8, v: usize, deg: usize, k: usize, out: *mut u32) -> usize {
    decode_list_g::<false>(d, v, deg, k, out)
}

/// Per-thread decode scratch with a one-list cache (`last`). Buffers are recycled through a
/// thread-local pool so that taking a cursor per work chunk does not allocate.
pub struct CompCur {
    buf: Vec<u32>,
    /// (store identity, node) of the list currently in buf[..len]; usize::MAX = none
    src: usize,
    last: usize,
    len: usize,
    // block-granularity scratch (BlockView)
    starts: Vec<u32>,
    /// m1-02: weights of the list in buf (weighted stores)
    wbuf: Vec<u32>,
}
thread_local! { static POOL: std::cell::RefCell<Vec<(Vec<u32>, Vec<u32>)>> = const { std::cell::RefCell::new(Vec::new()) }; }
impl CompCur {
    pub fn new() -> CompCur {
        let (buf, starts) = POOL.try_with(|p| p.borrow_mut().pop()).ok().flatten().unwrap_or_default();
        CompCur { buf, src: 0, last: usize::MAX, len: 0, starts, wbuf: Vec::new() }
    }
    /// m1-02: length of the cached list.
    #[inline(always)]
    pub fn len_of_last(&self) -> usize {
        self.len
    }
    /// m1-02: frees the decode buffers (and forgets the cached list).
    pub fn trim(&mut self) {
        self.buf = Vec::new();
        self.wbuf = Vec::new();
        self.last = usize::MAX;
        self.len = 0;
    }
    #[inline(always)]
    fn reserve(&mut self, k: usize) -> *mut u32 {
        if self.buf.len() < k + 4 {
            self.buf.resize((k + 16).next_power_of_two(), 0);
        }
        self.buf.as_mut_ptr()
    }
}
impl Default for CompCur {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for CompCur {
    fn drop(&mut self) {
        if self.buf.capacity() == 0 && self.starts.capacity() == 0 {
            return;
        }
        let b = (std::mem::take(&mut self.buf), std::mem::take(&mut self.starts));
        let _ = POOL.try_with(|p| {
            let mut p = p.borrow_mut();
            if p.len() < 64 {
                p.push(b);
            }
        });
    }
}

pub struct Comp {
    pub n: usize,
    pub entries: usize,
    pub data: Buf<u8>,
    pub data_len: usize, // without padding
    pub boff: EliasFano,
    pub chunks: Vec<(usize, usize)>,
    /// optional entry offsets (n+1 cumulative degrees), see DEG_INDEX
    pub eoff: Option<EliasFano>,
    dp: *const u8,
    /// m1-02: records carry a weight stream after the target stream
    pub weighted: bool,
    /// m1-02: largest degree (0 = unknown)
    pub max_deg: usize,
    /// m1-02: the largest degrees, descending (up to 64; empty = unknown, use max_deg)
    pub top_degs: Vec<u32>,
}
unsafe impl Send for Comp {}
unsafe impl Sync for Comp {}

pub struct EncodeStats {
    pub encode_s: f64,
    pub concat_s: f64,
    pub ef_s: f64,
}

impl Comp {
    /// Parallel encoder: one task per block. `list(v, out)` appends the sorted neighbors of v.
    pub fn build<L: Fn(usize, &mut Vec<u32>) + Sync>(n: usize, entries: usize, chunks: Vec<(usize, usize)>, list: L) -> (Comp, EncodeStats) {
        Comp::build_with(n, entries, chunks, || (), |_, v, out| list(v, out))
    }

    /// As `build`, with a per-task state created by `init` (e.g. a decode cursor of the source).
    pub fn build_with<S, I: Fn() -> S + Sync, L: Fn(&mut S, usize, &mut Vec<u32>) + Sync>(n: usize, entries: usize, chunks: Vec<(usize, usize)>, init: I, list: L) -> (Comp, EncodeStats) {
        let t = std::time::Instant::now();
        let parts: Vec<(Vec<u8>, Vec<u32>)> = chunks
            .par_iter()
            .map(|&(lo, hi)| {
                let mut st = init();
                let mut out = Vec::new();
                let mut rel = Vec::with_capacity(hi - lo);
                let mut l: Vec<u32> = Vec::new();
                let mut tmp: Vec<u32> = Vec::new();
                for v in lo..hi {
                    rel.push(out.len() as u32);
                    l.clear();
                    list(&mut st, v, &mut l);
                    put_varint(l.len() as u64, &mut out);
                    if l.is_empty() {
                        continue;
                    }
                    tmp.clear();
                    tmp.push(zz(l[0] as i64 - v as i64) as u32); // fits: |n0 - v| < 2^31
                    for w in l.windows(2) {
                        debug_assert!(w[1] > w[0]);
                        tmp.push(w[1] - w[0] - 1);
                    }
                    svb::encode(&tmp, &mut out);
                }
                (out, rel)
            })
            .collect();
        let encode_s = t.elapsed().as_secs_f64();
        let t = std::time::Instant::now();
        let total: usize = parts.iter().map(|p| p.0.len()).sum();
        assert!(parts.iter().all(|p| p.0.len() < u32::MAX as usize), "block larger than 4 GiB");
        let mut starts = Vec::with_capacity(parts.len() + 1);
        let mut acc = 0usize;
        for p in &parts {
            starts.push(acc);
            acc += p.0.len();
        }
        let mut data = vec![0u8; total + 64];
        let mut idx = vec![0u64; n + 1];
        {
            let dp = SyncPtr(data.as_mut_ptr());
            let ip = SyncPtr(idx.as_mut_ptr());
            parts.par_iter().zip(chunks.par_iter()).zip(starts.par_iter()).for_each(|((p, &(lo, _)), &base)| unsafe {
                std::ptr::copy_nonoverlapping(p.0.as_ptr(), dp.get().add(base), p.0.len());
                for (i, &r) in p.1.iter().enumerate() {
                    ip.write(lo + i, base as u64 + r as u64);
                }
            });
        }
        idx[n] = total as u64;
        drop(parts);
        let concat_s = t.elapsed().as_secs_f64();
        let t = std::time::Instant::now();
        let boff = EliasFano::build(&idx, total as u64);
        let ef_s = t.elapsed().as_secs_f64();
        let data = Buf::Vec(data);
        let dp = data.as_ptr();
        (Comp { n, entries, data, data_len: total, boff, chunks, eoff: None, dp, weighted: false, max_deg: 0, top_degs: Vec::new() }, EncodeStats { encode_s, concat_s, ef_s })
    }

    pub fn from_raw(g: &Raw) -> (Comp, EncodeStats) {
        Comp::build(g.n, g.targets.len(), g.chunks.clone(), |v, out| out.extend_from_slice(g.list(v)))
    }

    pub fn write(&self, dir: &Path, fsync: bool) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(dir)?;
        let mut f = std::fs::File::create(dir.join("adj.bin"))?;
        f.write_all(&self.data)?;
        if fsync {
            f.sync_all()?;
        }
        self.boff.write(&dir.join("boff.ef"))?;
        let mut b: Vec<u8> = Vec::with_capacity((self.chunks.len() + 1) * 4);
        for &(lo, _) in &self.chunks {
            b.extend_from_slice(&(lo as u32).to_le_bytes());
        }
        b.extend_from_slice(&(self.n as u32).to_le_bytes());
        std::fs::write(dir.join("blocks.u32"), &b)?;
        let meta = serde_json::json!({
            "format": "m1-13 prototype of DESIGN D4: delta + Stream VByte records in node-aligned blocks, Elias-Fano byte offsets",
            "n": self.n, "entries": self.entries, "data_bytes": self.data_len, "padding_bytes": 64,
            "boff_ef_bytes": self.boff.stored_bytes(), "blocks": self.chunks.len(), "block_entries_target": BLOCK_ENTRIES,
            "weighted": self.weighted, "max_deg": self.max_deg,
        });
        std::fs::write(dir.join("meta.json"), serde_json::to_string_pretty(&meta).unwrap())?;
        if fsync {
            for name in ["boff.ef", "blocks.u32", "meta.json"] {
                std::fs::File::open(dir.join(name))?.sync_all()?;
            }
            std::fs::File::open(dir)?.sync_all()?;
        }
        Ok(())
    }

    pub fn open(dir: &Path, mem: Mem) -> std::io::Result<Comp> {
        let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json"))?).unwrap();
        let n = meta["n"].as_u64().unwrap() as usize;
        let entries = meta["entries"].as_u64().unwrap() as usize;
        let data_len = meta["data_bytes"].as_u64().unwrap() as usize;
        let data: Buf<u8> = load_file(&dir.join("adj.bin"), mem, 0)?;
        assert_eq!(data.len(), data_len + 64);
        let boff = EliasFano::read(&dir.join("boff.ef"))?;
        assert_eq!(boff.n, n + 1);
        let b = std::fs::read(dir.join("blocks.u32"))?;
        let firsts: Vec<usize> = b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap()) as usize).collect();
        let chunks = firsts.windows(2).map(|w| (w[0], w[1])).collect();
        let dp = data.as_ptr();
        let weighted = meta["weighted"].as_bool().unwrap_or(false);
        let max_deg = meta["max_deg"].as_u64().unwrap_or(0) as usize;
        let mut c = Comp { n, entries, data, data_len, boff, chunks, eoff: None, dp, weighted, max_deg, top_degs: Vec::new() };
        if DEG_INDEX.load(Relaxed) {
            c.build_degree_index();
        }
        Ok(c)
    }

    pub fn build_degree_index(&mut self) {
        let n = self.n;
        let mut off = vec![0u64; n + 1];
        let op = SyncPtr(off.as_mut_ptr());
        self.chunks.par_iter().for_each(|&(lo, hi)| {
            self.for_range_deg(lo, hi, |v, d| unsafe { op.write(v + 1, d as u64) });
        });
        for v in 0..n {
            off[v + 1] += off[v];
        }
        assert_eq!(off[n] as usize, self.entries);
        self.eoff = Some(EliasFano::build(&off, self.entries as u64));
    }

    #[inline(always)]
    fn dptr(&self) -> *const u8 {
        self.dp
    }

    /// Sequential decode of the records of lo..hi, starting at byte `pos`: decode a batch of
    /// lists into the buffer (BATCH entries, at least one list), then hand them to `f` one by one.
    /// With BATCH = 1 this is the list-by-list loop (decode and kernel body interleaved).
    #[inline(always)]
    fn scan<F: FnMut(usize, &[u32])>(&self, lo: usize, hi: usize, mut pos: usize, c: &mut CompCur, mut f: F) -> usize {
        assert!(!self.weighted, "m1-02: sequential scan of a weighted store is not implemented");
        c.last = usize::MAX;
        let d = self.dptr();
        let batch = BATCH.load(Relaxed);
        if batch <= 1 {
            for v in lo..hi {
                unsafe {
                    let deg = get_varint(d, &mut pos);
                    if deg == 0 {
                        f(v, &[]);
                        continue;
                    }
                    let out = c.reserve(deg);
                    pos += decode_list(d.add(pos), v, deg, deg, out);
                    f(v, std::slice::from_raw_parts(out, deg));
                }
            }
            return pos;
        }
        let mut v = lo;
        unsafe {
            while v < hi {
                let first = v;
                let mut fill = 0usize;
                c.starts.clear();
                while v < hi && fill < batch {
                    let deg = get_varint(d, &mut pos);
                    c.starts.push(fill as u32);
                    if deg > 0 {
                        if c.buf.len() < fill + deg + 4 {
                            c.buf.resize((fill + deg + 16).next_power_of_two(), 0);
                        }
                        pos += decode_list(d.add(pos), v, deg, deg, c.buf.as_mut_ptr().add(fill));
                        fill += deg;
                    }
                    v += 1;
                }
                c.starts.push(fill as u32);
                for (i, u) in (first..v).enumerate() {
                    let (a, b) = (*c.starts.get_unchecked(i) as usize, *c.starts.get_unchecked(i + 1) as usize);
                    f(u, c.buf.get_unchecked(a..b));
                }
            }
        }
        pos
    }

    /// Configuration C: decode everything into uncompressed arrays in anonymous memory.
    /// Returns the graph and (seconds for the degree pass + prefix sum, seconds for the decode pass).
    pub fn decode_to_raw(&self) -> (Raw, f64, f64) {
        let t = std::time::Instant::now();
        let n = self.n;
        let mut offsets: Buf<u64> = anon_buf(n + 1);
        let op = SyncPtr(anon_ptr(&mut offsets));
        self.chunks.par_iter().for_each(|&(lo, hi)| {
            self.for_range_deg(lo, hi, |v, d| unsafe { op.write(v + 1, d as u64) });
        });
        {
            let o = unsafe { op.slice_mut(0, n + 1) };
            o[0] = 0;
            for v in 0..n {
                o[v + 1] += o[v];
            }
            assert_eq!(o[n] as usize, self.entries);
        }
        let deg_s = t.elapsed().as_secs_f64();
        let t = std::time::Instant::now();
        let mut targets: Buf<u32> = anon_buf(self.entries);
        let tp = SyncPtr(anon_ptr(&mut targets));
        let off = &offsets[..];
        self.chunks.par_iter().for_each(|&(lo, hi)| unsafe {
            let d = self.dptr();
            let mut pos = self.boff.get(lo) as usize;
            for v in lo..hi {
                let deg = get_varint(d, &mut pos);
                if deg > 0 {
                    pos += decode_list_exact(d.add(pos), v, deg, deg, tp.get().add(off[v] as usize));
                }
            }
        });
        let dec_s = t.elapsed().as_secs_f64();
        (Raw::from_bufs(offsets, targets), deg_s, dec_s)
    }
}

impl Adj for Comp {
    type Cur = CompCur;
    fn kind(&self) -> &'static str {
        "comp"
    }
    #[inline(always)]
    fn cur(&self) -> CompCur {
        CompCur::new()
    }
    #[inline(always)]
    fn n(&self) -> usize {
        self.n
    }
    #[inline(always)]
    fn entries(&self) -> usize {
        self.entries
    }
    #[inline(always)]
    fn chunks(&self) -> &[(usize, usize)] {
        &self.chunks
    }
    #[inline(always)]
    fn deg(&self, v: usize) -> usize {
        if let Some(e) = &self.eoff {
            let (a, b) = e.get2(v);
            return (b - a) as usize;
        }
        let mut pos = self.boff.get(v) as usize;
        unsafe { get_varint(self.dptr(), &mut pos) }
    }
    #[inline(always)]
    fn nbrs<'a>(&'a self, v: usize, c: &'a mut CompCur) -> &'a [u32] {
        if c.last == v && c.src == self.dptr() as usize {
            return unsafe { c.buf.get_unchecked(..c.len) };
        }
        let mut pos = self.boff.get(v) as usize;
        unsafe {
            let d = self.dptr();
            let deg = get_varint(d, &mut pos);
            if deg > 0 {
                let out = c.reserve(deg);
                decode_list(d.add(pos), v, deg, deg, out);
            }
            c.last = v;
            c.src = self.dptr() as usize;
            c.len = deg;
            c.buf.get_unchecked(..deg)
        }
    }
    #[inline(always)]
    fn prefix<'a>(&'a self, v: usize, k: usize, c: &'a mut CompCur) -> (&'a [u32], usize) {
        let mut pos = self.boff.get(v) as usize;
        unsafe {
            let d = self.dptr();
            let deg = get_varint(d, &mut pos);
            let k = k.min(deg);
            c.last = usize::MAX;
            if k > 0 {
                let out = c.reserve(k);
                decode_list(d.add(pos), v, deg, k, out);
            }
            (c.buf.get_unchecked(..k), deg)
        }
    }
    #[inline(always)]
    fn for_range<F: FnMut(usize, &[u32])>(&self, lo: usize, hi: usize, c: &mut CompCur, f: F) {
        if lo < hi {
            self.scan(lo, hi, self.boff.get(lo) as usize, c, f);
        }
    }
    #[inline(always)]
    fn for_range_prefix<F: FnMut(usize, &[u32], usize)>(&self, lo: usize, hi: usize, k: usize, c: &mut CompCur, mut f: F) {
        if lo >= hi {
            return;
        }
        c.last = usize::MAX;
        let d = self.dptr();
        let mut it = self.boff.iter_from(lo);
        let pb = PREFIX_BATCH.load(Relaxed);
        if pb > 1 {
            // decode the prefixes of `pb` nodes first, then run the kernel body over them
            let slot = (k.max(1) + 3) / 4 * 4 + 4;
            let out = c.reserve(pb * slot);
            let mut v = lo;
            unsafe {
                while v < hi {
                    let first = v;
                    c.starts.clear();
                    while v < hi && v - first < pb {
                        let mut pos = it.next() as usize;
                        let deg = get_varint(d, &mut pos);
                        let kk = k.min(deg);
                        if kk > 0 {
                            decode_list(d.add(pos), v, deg, kk, out.add((v - first) * slot));
                        }
                        c.starts.push(deg as u32);
                        v += 1;
                    }
                    for (i, u) in (first..v).enumerate() {
                        let deg = *c.starts.get_unchecked(i) as usize;
                        f(u, std::slice::from_raw_parts(out.add(i * slot), k.min(deg)), deg);
                    }
                }
            }
            return;
        }
        let out = c.reserve(k.max(1));
        for v in lo..hi {
            let mut pos = it.next() as usize;
            unsafe {
                let deg = get_varint(d, &mut pos);
                let kk = k.min(deg);
                if kk > 0 {
                    decode_list(d.add(pos), v, deg, kk, out);
                }
                f(v, std::slice::from_raw_parts(out, kk), deg);
            }
        }
    }
    #[inline(always)]
    fn for_range_deg<F: FnMut(usize, usize)>(&self, lo: usize, hi: usize, mut f: F) {
        if lo >= hi {
            return;
        }
        if let Some(e) = &self.eoff {
            let mut it = e.iter_from(lo);
            let mut a = it.next();
            for v in lo..hi {
                let b = it.next();
                f(v, (b - a) as usize);
                a = b;
            }
            return;
        }
        let d = self.dptr();
        let mut it = self.boff.iter_from(lo);
        for v in lo..hi {
            let mut pos = it.next() as usize;
            f(v, unsafe { get_varint(d, &mut pos) });
        }
    }
    /// Early exit: decode the first 16 neighbors, test them, and only then decode the whole list.
    #[inline(always)]
    fn any<F: FnMut(u32) -> bool>(&self, v: usize, c: &mut CompCur, mut f: F) -> (bool, usize) {
        const HEAD: usize = 16;
        let mut pos = self.boff.get(v) as usize;
        unsafe {
            let d = self.dptr();
            let deg = get_varint(d, &mut pos);
            if deg == 0 {
                return (false, 0);
            }
            c.last = usize::MAX;
            let k = HEAD.min(deg);
            let out = c.reserve(deg.min(HEAD));
            decode_list(d.add(pos), v, deg, k, out);
            for i in 0..k {
                if f(*out.add(i)) {
                    return (true, i + 1);
                }
            }
            if deg > k {
                let out = c.reserve(deg);
                decode_list(d.add(pos), v, deg, deg, out);
                for i in k..deg {
                    if f(*out.add(i)) {
                        return (true, i + 1);
                    }
                }
            }
            (false, deg)
        }
    }
}

// ---------------------------------------------------------------- m1-02: weighted records, writer
/// Appends the record of node v to `out`: [deg LEB128][svb zigzag(first - v), gaps - 1]
/// [svb weight - 1 (weighted only)]. `targets` strictly increasing; weights >= 1.
pub fn encode_record(v: usize, targets: &[u32], weights: Option<&[u32]>, out: &mut Vec<u8>, tmp: &mut Vec<u32>) {
    put_varint(targets.len() as u64, out);
    if targets.is_empty() {
        return;
    }
    tmp.clear();
    tmp.push(zz(targets[0] as i64 - v as i64) as u32);
    for w in targets.windows(2) {
        debug_assert!(w[1] > w[0], "targets not strictly increasing");
        tmp.push(w[1] - w[0] - 1);
    }
    svb::encode(tmp, out);
    if let Some(ws) = weights {
        debug_assert_eq!(ws.len(), targets.len());
        tmp.clear();
        tmp.extend(ws.iter().map(|&w| {
            debug_assert!(w >= 1);
            w - 1
        }));
        svb::encode(tmp, out);
    }
}

/// Node-aligned blocks, exactly as `edge_chunks(offsets, BLOCK_ENTRIES)` but from degrees: a
/// block starting at a ends at the first b in a+1..=min(a + 2^16, n) with offsets[b] >=
/// offsets[a] + BLOCK_ENTRIES, else at min(a + 2^16, n).
pub fn chunks_from_degrees(deg: &[u32]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let n = deg.len();
    let mut a = 0usize;
    while a < n {
        let hi = (a + (1 << 16)).min(n);
        let mut b = a + 1;
        let mut e = deg[a] as u64; // offsets[b] - offsets[a]
        while b < hi && e < BLOCK_ENTRIES {
            e += deg[b] as u64;
            b += 1;
        }
        out.push((a, b));
        a = b;
    }
    out
}

impl Comp {
    /// Weighted neighbours of v (random access): (targets, weights). Weighted stores only.
    #[inline(always)]
    pub fn nbrs_w<'a>(&'a self, v: usize, c: &'a mut CompCur) -> (&'a [u32], &'a [u32]) {
        debug_assert!(self.weighted);
        if c.last == v && c.src == self.dptr() as usize {
            return unsafe { (c.buf.get_unchecked(..c.len), c.wbuf.get_unchecked(..c.len)) };
        }
        let mut pos = self.boff.get(v) as usize;
        unsafe {
            let d = self.dptr();
            let deg = get_varint(d, &mut pos);
            if deg > 0 {
                let out = c.reserve(deg);
                let used = decode_list(d.add(pos), v, deg, deg, out);
                if c.wbuf.len() < deg + 4 {
                    c.wbuf.resize((deg + 16).next_power_of_two(), 0);
                }
                let wo = c.wbuf.as_mut_ptr();
                svb::decode(d.add(pos + used), deg, deg, wo);
                for i in 0..deg {
                    *wo.add(i) += 1;
                }
            }
            c.last = v;
            c.src = self.dptr() as usize;
            c.len = deg;
            (c.buf.get_unchecked(..deg), c.wbuf.get_unchecked(..deg))
        }
    }

    /// madvise on the mapped data (no-op for heap data).
    pub fn advise(&self, a: memmap2::Advice) {
        if let Buf::Mmap(m, _) = &self.data {
            let _ = m.advise(a);
        }
    }

    /// Bytes of the in-memory index (Elias-Fano offsets + blocks), anonymous memory.
    pub fn index_ram_bytes(&self) -> u64 {
        (self.boff.ram_bytes() + self.chunks.len() * 16) as u64
    }
}

/// Streaming writer: blocks of encoded records are appended in node order to `path`; only the
/// n + 1 byte offsets (8 bytes per node) and the degrees (4 bytes per node) stay in memory.
pub struct StoreWriter {
    file: std::fs::File,
    path: std::path::PathBuf,
    pub n: usize,
    pos: u64,
    next_node: usize,
    pub idx: Vec<u64>,
    pub deg: Vec<u32>,
    pub entries: u64,
    pub weighted: bool,
}
impl StoreWriter {
    pub fn create(path: &Path, n: usize, weighted: bool) -> std::io::Result<StoreWriter> {
        let file = std::fs::File::options().read(true).write(true).create(true).truncate(true).open(path)?;
        Ok(StoreWriter { file, path: path.to_path_buf(), n, pos: 0, next_node: 0, idx: vec![0u64; n + 1], deg: vec![0u32; n], entries: 0, weighted })
    }
    /// Memory this writer holds (offsets and degrees).
    pub fn ram_bytes(n: usize) -> u64 {
        (n as u64 + 1) * 8 + n as u64 * 4
    }
    /// Reserves the next block: nodes lo..hi (lo must be the next node), `bytes` long, and
    /// returns the file offset where it goes. `rel` = byte offset of each node inside the block.
    pub fn place(&mut self, lo: usize, hi: usize, bytes: usize, rel: &[u32], deg: &[u32]) -> u64 {
        assert_eq!(lo, self.next_node);
        assert_eq!(rel.len(), hi - lo);
        let base = self.pos;
        for i in 0..hi - lo {
            self.idx[lo + i] = base + rel[i] as u64;
            self.deg[lo + i] = deg[i];
            self.entries += deg[i] as u64;
        }
        self.pos += bytes as u64;
        self.next_node = hi;
        base
    }
    /// A second handle of the file, for writing outside a lock on the writer.
    pub fn file_handle(&self) -> std::fs::File {
        self.file.try_clone().expect("dup")
    }
    /// Writes bytes at an offset returned by `place` (may be called from several threads).
    pub fn write_at(&self, off: u64, data: &[u8]) {
        use std::os::unix::fs::FileExt;
        self.file.write_all_at(data, off).expect("store write");
    }
    /// Pads, builds the Elias-Fano index and maps the file read-only (no pre-fault).
    /// `keep_file` false: the file is unlinked once mapped (a spilled quotient graph then lives
    /// exactly as long as its mapping, and nothing is left behind if the process dies).
    pub fn finish(mut self, keep_file: bool) -> std::io::Result<Comp> {
        use std::os::unix::fs::FileExt;
        assert_eq!(self.next_node, self.n);
        let data_len = self.pos as usize;
        self.file.write_all_at(&[0u8; 64], self.pos)?;
        self.idx[self.n] = self.pos;
        let boff = EliasFano::build(&self.idx, self.pos);
        self.idx = Vec::new();
        let chunks = chunks_from_degrees(&self.deg);
        let max_deg = self.deg.iter().copied().max().unwrap_or(0) as usize;
        let mut top_degs = self.deg.clone();
        let k = 64.min(top_degs.len());
        if k > 0 && k < top_degs.len() {
            top_degs.select_nth_unstable_by(k - 1, |a, b| b.cmp(a));
        }
        top_degs.truncate(k);
        top_degs.sort_unstable_by(|a, b| b.cmp(a));
        self.deg = Vec::new();
        let m = unsafe { memmap2::Mmap::map(&self.file)? };
        let data = Buf::Mmap(m, data_len + 64);
        if !keep_file {
            std::fs::remove_file(&self.path)?;
        }
        let dp = data.as_ptr();
        Ok(Comp { n: self.n, entries: self.entries as usize, data, data_len, boff, chunks, eoff: None, dp, weighted: self.weighted, max_deg, top_degs })
    }
    /// For a store meant to be reopened with `Comp::open`: writes boff.ef, blocks.u32 and meta.json
    /// next to adj.bin (the writer's file must be `dir/adj.bin`).
    pub fn finish_dir(self, dir: &Path, fsync: bool) -> std::io::Result<Comp> {
        let f2 = self.file.try_clone()?;
        let c = self.finish(true)?;
        c.boff.write(&dir.join("boff.ef"))?;
        let mut b: Vec<u8> = Vec::with_capacity((c.chunks.len() + 1) * 4);
        for &(lo, _) in &c.chunks {
            b.extend_from_slice(&(lo as u32).to_le_bytes());
        }
        b.extend_from_slice(&(c.n as u32).to_le_bytes());
        std::fs::write(dir.join("blocks.u32"), &b)?;
        let meta = serde_json::json!({
            "format": "m1-13 prototype of DESIGN D4: delta + Stream VByte records in node-aligned blocks, Elias-Fano byte offsets (written by m1-02's streaming writer)",
            "n": c.n, "entries": c.entries, "data_bytes": c.data_len, "padding_bytes": 64,
            "boff_ef_bytes": c.boff.stored_bytes(), "blocks": c.chunks.len(), "block_entries_target": BLOCK_ENTRIES,
            "weighted": c.weighted, "max_deg": c.max_deg,
        });
        std::fs::write(dir.join("meta.json"), serde_json::to_string_pretty(&meta).unwrap())?;
        if fsync {
            f2.sync_all()?;
            for name in ["boff.ef", "blocks.u32", "meta.json"] {
                std::fs::File::open(dir.join(name))?.sync_all()?;
            }
            std::fs::File::open(dir)?.sync_all()?;
        }
        Ok(c)
    }
}

/// Ablation: the same store accessed as if the index held one offset per ~64K-entry block instead
/// of one per node. Sequential access is unchanged; random access (deg, nbrs, prefix, any) walks
/// the block from its start, skipping records by summing control bytes, until it reaches the node;
/// prefix scans decode whole lists.
pub struct BlockView<'a>(pub &'a Comp);

impl BlockView<'_> {
    #[inline(always)]
    fn seek(&self, v: usize) -> (usize, usize) {
        let g = self.0;
        let b = g.chunks.partition_point(|&(lo, _)| lo <= v) - 1;
        let lo = g.chunks[b].0;
        let d = g.dptr();
        let mut pos = g.boff.get(lo) as usize;
        unsafe {
            for _ in lo..v {
                let deg = get_varint(d, &mut pos);
                if deg > 0 {
                    pos += svb::stream_len(d.add(pos), deg);
                }
            }
            let deg = get_varint(d, &mut pos);
            (pos, deg)
        }
    }
}

impl Adj for BlockView<'_> {
    type Cur = CompCur;
    fn kind(&self) -> &'static str {
        "comp-block"
    }
    fn cur(&self) -> CompCur {
        CompCur::new()
    }
    fn n(&self) -> usize {
        self.0.n
    }
    fn entries(&self) -> usize {
        self.0.entries
    }
    fn chunks(&self) -> &[(usize, usize)] {
        &self.0.chunks
    }
    #[inline(always)]
    fn deg(&self, v: usize) -> usize {
        self.seek(v).1
    }
    #[inline(always)]
    fn nbrs<'a>(&'a self, v: usize, c: &'a mut CompCur) -> &'a [u32] {
        let (pos, deg) = self.seek(v);
        c.last = usize::MAX;
        unsafe {
            if deg > 0 {
                let out = c.reserve(deg);
                decode_list(self.0.dptr().add(pos), v, deg, deg, out);
            }
            c.buf.get_unchecked(..deg)
        }
    }
    #[inline(always)]
    fn prefix<'a>(&'a self, v: usize, k: usize, c: &'a mut CompCur) -> (&'a [u32], usize) {
        let (pos, deg) = self.seek(v);
        let k = k.min(deg);
        c.last = usize::MAX;
        unsafe {
            if k > 0 {
                let out = c.reserve(k);
                decode_list(self.0.dptr().add(pos), v, deg, k, out);
            }
            (c.buf.get_unchecked(..k), deg)
        }
    }
    #[inline(always)]
    fn for_range<F: FnMut(usize, &[u32])>(&self, lo: usize, hi: usize, c: &mut CompCur, f: F) {
        self.0.for_range(lo, hi, c, f)
    }
    #[inline(always)]
    fn for_range_prefix<F: FnMut(usize, &[u32], usize)>(&self, lo: usize, hi: usize, k: usize, c: &mut CompCur, mut f: F) {
        self.for_range(lo, hi, c, |v, l| f(v, &l[..k.min(l.len())], l.len()));
    }
    #[inline(always)]
    fn for_range_deg<F: FnMut(usize, usize)>(&self, lo: usize, hi: usize, mut f: F) {
        if lo >= hi {
            return;
        }
        let g = self.0;
        let d = g.dptr();
        let mut pos = g.boff.get(lo) as usize;
        unsafe {
            for v in lo..hi {
                let deg = get_varint(d, &mut pos);
                if deg > 0 {
                    pos += svb::stream_len(d.add(pos), deg);
                }
                f(v, deg);
            }
        }
    }
    #[inline(always)]
    fn any<F: FnMut(u32) -> bool>(&self, v: usize, c: &mut CompCur, mut f: F) -> (bool, usize) {
        let l = self.nbrs(v, c);
        for (i, &u) in l.iter().enumerate() {
            if f(u) {
                return (true, i + 1);
            }
        }
        (false, l.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn toy(n: usize, seed: u64) -> Raw {
        let mut x = seed;
        let mut offsets = vec![0u64];
        let mut targets = Vec::new();
        for v in 0..n {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let deg = match x % 10 {
                0 => 0,
                1 => 1,
                2 => 3,
                3 => 4,
                4 => 5,
                5 => 17,
                6 => 200,
                _ => (x >> 8) % 40,
            } as usize;
            let mut l: Vec<u32> = (0..deg)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    (x % if v % 7 == 0 { 1 << 31 } else { n as u64 * 4 }) as u32
                })
                .collect();
            l.sort_unstable();
            l.dedup();
            targets.extend_from_slice(&l);
            offsets.push(targets.len() as u64);
        }
        Raw::from_bufs(Buf::Vec(offsets), Buf::Vec(targets))
    }
    #[test]
    fn roundtrip_all_paths() {
        for dec in [0u8, 1, 2] {
            DECODER.store(dec, Relaxed);
            let g = toy(3000, 0x9E3779B97F4A7C15);
            let (c, _) = Comp::from_raw(&g);
            let mut cur = c.cur();
            for v in 0..g.n {
                assert_eq!(c.deg(v), g.deg(v));
                assert_eq!(c.nbrs(v, &mut cur), g.list(v), "nbrs {v}");
                for k in [0, 1, 2, 4, 5, 16, 1000] {
                    let (p, d) = c.prefix(v, k, &mut cur);
                    assert_eq!((p, d), (&g.list(v)[..k.min(g.deg(v))], g.deg(v)));
                }
                let target = g.list(v).get(g.deg(v) / 2).copied();
                let (found, ex) = c.any(v, &mut cur, |u| Some(u) == target);
                assert_eq!((found, ex), g.any(v, &mut (), |u| Some(u) == target));
            }
            for &(lo, hi) in c.chunks() {
                let mut v0 = lo;
                c.for_range(lo, hi, &mut cur, |v, l| {
                    assert_eq!(v, v0);
                    assert_eq!(l, g.list(v));
                    v0 += 1;
                });
                assert_eq!(v0, hi);
                c.for_range_prefix(lo, hi, 2, &mut cur, |v, l, d| assert_eq!((l, d), (&g.list(v)[..2.min(g.deg(v))], g.deg(v))));
                c.for_range_deg(lo, hi, |v, d| assert_eq!(d, g.deg(v)));
                let bv = BlockView(&c);
                let mut v0 = lo;
                bv.for_range(lo, hi, &mut cur, |v, l| {
                    assert_eq!(v, v0);
                    assert_eq!(l, g.list(v));
                    v0 += 1;
                });
                bv.for_range_deg(lo, hi, |v, d| assert_eq!(d, g.deg(v)));
            }
            let bv = BlockView(&c);
            for v in (0..g.n).step_by(7) {
                assert_eq!(bv.nbrs(v, &mut cur), g.list(v));
                assert_eq!(bv.deg(v), g.deg(v));
            }
            let (r, _, _) = c.decode_to_raw();
            assert_eq!(&r.offsets[..], &g.offsets[..]);
            assert_eq!(&r.targets[..], &g.targets[..]);
            // every batch size of the sequential path, and the optional degree index
            for batch in [1usize, 2, 7, 100, 8192, usize::MAX] {
                BATCH.store(batch, Relaxed);
                let mut v0 = 0;
                c.for_range(0, g.n, &mut cur, |v, l| {
                    assert_eq!(v, v0);
                    assert_eq!(l, g.list(v), "batch {batch} v {v}");
                    v0 += 1;
                });
                assert_eq!(v0, g.n);
            }
            BATCH.store(8192, Relaxed);
            for pb in [3usize, 256] {
                PREFIX_BATCH.store(pb, Relaxed);
                for k in [0usize, 1, 2, 5, 9] {
                    let mut v0 = 0;
                    c.for_range_prefix(0, g.n, k, &mut cur, |v, l, d| {
                        assert_eq!(v, v0);
                        assert_eq!((l, d), (&g.list(v)[..k.min(g.deg(v))], g.deg(v)), "prefix batch {pb} k {k} v {v}");
                        v0 += 1;
                    });
                    assert_eq!(v0, g.n);
                }
            }
            PREFIX_BATCH.store(0, Relaxed);
            let mut c = c;
            c.build_degree_index();
            for v in 0..g.n {
                assert_eq!(c.deg(v), g.deg(v));
            }
            for &(lo, hi) in c.chunks() {
                c.for_range_deg(lo, hi, |v, d| assert_eq!(d, g.deg(v)));
            }
        }
        DECODER.store(2, Relaxed);
    }
}
