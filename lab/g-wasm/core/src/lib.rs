//! graphcore: minimal "same core" prototype — edge-list parser, CSR builder,
//! pull PageRank and connected components.
//!
//! Portability rules followed here (these are the point of the experiment):
//! * no std::fs / mmap / threads in this crate: all I/O is the caller's job
//!   (bytes in, arrays out);
//! * parallelism only through the `par` module; with feature `parallel` it is
//!   rayon, without it it is plain loops (same code paths, same results);
//! * all indices that can exceed 2^32 are u64 (edge offsets), node ids are u32;
//!   nothing assumes usize == 64 bits (casts are checked where it matters);
//! * allocations are sized exactly up front (no Vec doubling) so peak memory
//!   is predictable inside a 4 GiB wasm32 address space.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};

pub mod par {
    use std::ops::Range;

    #[cfg(feature = "parallel")]
    pub fn threads() -> usize {
        rayon::current_num_threads()
    }
    #[cfg(not(feature = "parallel"))]
    pub fn threads() -> usize {
        1
    }

    // NOTE (measured): wasm engines have no on-stack replacement. V8 runs a
    // function in its baseline tier (Liftoff) until a *later call* picks up the
    // optimized TurboFan code, so a one-shot kernel can run 2-2.7x slower on
    // first use. A non-inlined per-chunk trampoline did NOT fix this in V8 14.x
    // (tried, reverted); see REPORT.md.
    /// Split 0..n into chunks of ~grain and run f on each (in parallel if enabled).
    pub fn for_ranges<F: Fn(Range<usize>) + Sync + Send>(n: usize, grain: usize, f: F) {
        let grain = grain.max(1);
        let nchunks = n.div_ceil(grain);
        let run = |c: usize| {
            let s = c * grain;
            f(s..(s + grain).min(n))
        };
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            (0..nchunks).into_par_iter().for_each(run);
        }
        #[cfg(not(feature = "parallel"))]
        (0..nchunks).for_each(run);
    }

    /// Map chunk index -> T, collected in order.
    pub fn map_collect<T: Send, F: Fn(usize) -> T + Sync + Send>(nchunks: usize, f: F) -> Vec<T> {
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            (0..nchunks).into_par_iter().map(f).collect()
        }
        #[cfg(not(feature = "parallel"))]
        (0..nchunks).map(f).collect()
    }

    /// Process disjoint mutable chunks of a slice.
    pub fn chunks_mut<T: Send, F: Fn(usize, &mut [T]) + Sync + Send>(s: &mut [T], grain: usize, f: F) {
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            s.par_chunks_mut(grain.max(1)).enumerate().for_each(|(i, c)| f(i, c));
        }
        #[cfg(not(feature = "parallel"))]
        s.chunks_mut(grain.max(1)).enumerate().for_each(|(i, c)| f(i, c));
    }
}

/// Raw pointer wrapper for disjoint parallel writes.
#[derive(Clone, Copy)]
struct SyncPtr<T>(*mut T);
unsafe impl<T> Send for SyncPtr<T> {}
unsafe impl<T> Sync for SyncPtr<T> {}
impl<T> SyncPtr<T> {
    #[inline(always)]
    unsafe fn write(self, i: usize, v: T) {
        self.0.add(i).write(v)
    }
}

fn into_plain_u32(v: Vec<AtomicU32>) -> Vec<u32> {
    let mut v = std::mem::ManuallyDrop::new(v);
    unsafe { Vec::from_raw_parts(v.as_mut_ptr() as *mut u32, v.len(), v.capacity()) }
}

// ---------------------------------------------------------------- parsing

#[derive(Default)]
pub struct EdgeList {
    pub src: Vec<u32>,
    pub dst: Vec<u32>,
}
impl EdgeList {
    pub fn len(&self) -> usize {
        self.src.len()
    }
    pub fn is_empty(&self) -> bool {
        self.src.is_empty()
    }
    pub fn with_capacity(c: usize) -> Self {
        EdgeList { src: Vec::with_capacity(c), dst: Vec::with_capacity(c) }
    }
}

const PARSE_GRAIN: usize = 4 << 20; // 4 MiB of text per task

/// Chunk boundaries aligned to the byte after a '\n'.
fn line_chunks(buf: &[u8], grain: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut s = 0;
    while s < buf.len() {
        let mut e = (s + grain).min(buf.len());
        while e < buf.len() && buf[e - 1] != b'\n' {
            e += 1;
        }
        out.push((s, e));
        s = e;
    }
    out
}

#[inline]
fn is_data_line_start(b: u8) -> bool {
    b.is_ascii_digit()
}

/// Number of data lines (first byte is a digit) in a newline-aligned chunk.
fn count_lines(c: &[u8]) -> usize {
    let mut n = 0usize;
    let mut at_start = true;
    for &b in c {
        if at_start && is_data_line_start(b) {
            n += 1;
        }
        at_start = b == b'\n';
    }
    n
}

#[inline(always)]
fn parse_uint(c: &[u8], i: &mut usize) -> u32 {
    let mut v: u64 = 0;
    while *i < c.len() {
        let d = c[*i].wrapping_sub(b'0');
        if d > 9 {
            break;
        }
        v = v * 10 + d as u64;
        *i += 1;
    }
    assert!(v <= u32::MAX as u64, "node id exceeds u32");
    v as u32
}

/// Parse "u<ws>v\n" lines of a newline-aligned chunk into out slices.
fn parse_chunk(c: &[u8], src: &mut [u32], dst: &mut [u32]) -> usize {
    let mut i = 0;
    let mut k = 0;
    let n = c.len();
    while i < n {
        if is_data_line_start(c[i]) {
            let u = parse_uint(c, &mut i);
            while i < n && (c[i] == b' ' || c[i] == b'\t' || c[i] == b',') {
                i += 1;
            }
            let v = parse_uint(c, &mut i);
            src[k] = u;
            dst[k] = v;
            k += 1;
        }
        // skip to next line
        while i < n && c[i] != b'\n' {
            i += 1;
        }
        i += 1;
    }
    k
}

/// Append all edges of a text buffer (must end at a line boundary, or be the
/// end of the input) to `el`. Two passes (count, then parse into exact slots),
/// so the edge arrays are never over-allocated.
pub fn parse_into(buf: &[u8], el: &mut EdgeList) {
    let chunks = line_chunks(buf, PARSE_GRAIN);
    let counts = par::map_collect(chunks.len(), |i| count_lines(&buf[chunks[i].0..chunks[i].1]));
    let total: usize = counts.iter().sum();
    let base = el.len();
    el.src.reserve_exact(total);
    el.dst.reserve_exact(total);
    unsafe {
        el.src.set_len(base + total);
        el.dst.set_len(base + total);
    }
    let mut starts = Vec::with_capacity(counts.len());
    let mut acc = base;
    for &c in &counts {
        starts.push(acc);
        acc += c;
    }
    let sp = SyncPtr(el.src.as_mut_ptr());
    let dp = SyncPtr(el.dst.as_mut_ptr());
    par::for_ranges(chunks.len(), 1, |r| {
        for ci in r {
            let (s, e) = chunks[ci];
            let off = starts[ci];
            let cnt = counts[ci];
            let (s_sl, d_sl) = unsafe {
                (
                    std::slice::from_raw_parts_mut({ sp }.0.add(off), cnt),
                    std::slice::from_raw_parts_mut({ dp }.0.add(off), cnt),
                )
            };
            let k = parse_chunk(&buf[s..e], s_sl, d_sl);
            assert_eq!(k, cnt);
        }
    });
}

pub fn parse_edges(buf: &[u8]) -> EdgeList {
    let mut el = EdgeList::default();
    parse_into(buf, &mut el);
    el
}

/// Streaming parser: push arbitrary byte chunks (e.g. from File.stream()).
#[derive(Default)]
pub struct StreamParser {
    carry: Vec<u8>,
    pub edges: EdgeList,
}
impl StreamParser {
    pub fn with_capacity(edges: usize) -> Self {
        StreamParser { carry: Vec::new(), edges: EdgeList::with_capacity(edges) }
    }
    pub fn push(&mut self, chunk: &[u8]) {
        // complete the carried partial line
        let mut start = 0;
        if !self.carry.is_empty() {
            match chunk.iter().position(|&b| b == b'\n') {
                Some(p) => {
                    self.carry.extend_from_slice(&chunk[..=p]);
                    let c = std::mem::take(&mut self.carry);
                    parse_into(&c, &mut self.edges);
                    start = p + 1;
                }
                None => {
                    self.carry.extend_from_slice(chunk);
                    return;
                }
            }
        }
        let rest = &chunk[start..];
        let last_nl = rest.iter().rposition(|&b| b == b'\n');
        match last_nl {
            Some(p) => {
                parse_into(&rest[..=p], &mut self.edges);
                self.carry.extend_from_slice(&rest[p + 1..]);
            }
            None => self.carry.extend_from_slice(rest),
        }
    }
    pub fn finish(mut self) -> EdgeList {
        if !self.carry.is_empty() {
            let c = std::mem::take(&mut self.carry);
            parse_into(&c, &mut self.edges);
        }
        self.edges
    }
}

// ---------------------------------------------------------------- CSR

/// Edge-offset type. usize on purpose: 8 B/node natively, 4 B/node on wasm32
/// (where > 2^32 entries cannot exist anyway). On-disk format stays u64.
pub type Ix = usize;

pub struct Csr {
    pub offsets: Vec<Ix>,  // n+1
    pub targets: Vec<u32>, // symmetric, sorted, dedup, no self loops
}
impl Csr {
    pub fn n(&self) -> usize {
        self.offsets.len() - 1
    }
    pub fn entries(&self) -> u64 {
        *self.offsets.last().unwrap() as u64
    }
    #[inline(always)]
    pub fn nbrs(&self, v: usize) -> &[u32] {
        &self.targets[self.offsets[v]..self.offsets[v + 1]]
    }
    pub fn bytes(&self) -> usize {
        self.offsets.len() * std::mem::size_of::<Ix>() + self.targets.len() * 4
    }
    /// From glab-csr-v0 u64 offsets (checked narrowing on 32-bit targets).
    pub fn from_u64_offsets(off: &[u64], targets: Vec<u32>) -> Csr {
        let offsets = off.iter().map(|&o| Ix::try_from(o).expect("offset overflows usize")).collect();
        Csr { offsets, targets }
    }
}

const NODE_GRAIN: usize = 1 << 14;
const EDGE_GRAIN: usize = 1 << 18;

fn par_max(el: &EdgeList) -> u32 {
    let m = el.len();
    par::map_collect(m.div_ceil(EDGE_GRAIN), |c| {
        let r = c * EDGE_GRAIN..((c + 1) * EDGE_GRAIN).min(m);
        let a = el.src[r.clone()].iter().copied().max().unwrap_or(0);
        let b = el.dst[r].iter().copied().max().unwrap_or(0);
        a.max(b)
    })
    .into_iter()
    .max()
    .unwrap_or(0)
}

/// Two-phase undirected CSR builder. Phase 1 (`count`) sees every edge once
/// and only keeps a degree array; phase 2 (`scatter`) sees every edge again
/// and writes targets. Edges may be fed in arbitrary batches, so the input
/// never has to be resident: a re-readable source (File, OPFS file, disk)
/// is streamed twice. Peak memory ~= targets (8 B/undirected edge) +
/// O(n) arrays, i.e. half of "edge list + CSR".
#[derive(Default)]
pub struct CsrBuilder {
    deg: Vec<AtomicU32>,       // phase 1, original id space
    remap: Vec<u32>,           // phase 2, original id -> dense
    ids: Vec<u32>,             // dense -> original id
    cursor: Vec<AtomicUsize>,  // phase 2: n+1, cursor[v] = next write slot of v
    targets: Vec<u32>,
}
impl CsrBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn count(&mut self, el: &EdgeList) {
        let mx = par_max(el) as usize;
        if mx + 1 > self.deg.len() {
            let old = std::mem::take(&mut self.deg);
            let mut plain = into_plain_u32(old);
            plain.resize(mx + 1, 0);
            let mut p = std::mem::ManuallyDrop::new(plain);
            self.deg = unsafe { Vec::from_raw_parts(p.as_mut_ptr() as *mut AtomicU32, p.len(), p.capacity()) };
        }
        let deg = &self.deg;
        par::for_ranges(el.len(), EDGE_GRAIN, |r| {
            for i in r {
                let (u, v) = (el.src[i], el.dst[i]);
                if u != v {
                    deg[u as usize].fetch_add(1, Relaxed);
                    deg[v as usize].fetch_add(1, Relaxed);
                }
            }
        });
    }
    /// End of phase 1: dense ids (degree > 0, increasing original id), offsets.
    pub fn finish_count(&mut self) {
        let deg = into_plain_u32(std::mem::take(&mut self.deg));
        let n = deg.iter().filter(|&&d| d > 0).count();
        let mut remap = vec![u32::MAX; deg.len()];
        let mut ids = Vec::with_capacity(n);
        let mut cursor = Vec::with_capacity(n + 1);
        let mut acc: u64 = 0;
        for (old, &d) in deg.iter().enumerate() {
            if d > 0 {
                remap[old] = ids.len() as u32;
                ids.push(old as u32);
                cursor.push(AtomicUsize::new(acc as usize));
                acc += d as u64;
            }
        }
        assert!(acc <= Ix::MAX as u64 / 4, "CSR does not fit this address space");
        cursor.push(AtomicUsize::new(acc as usize));
        drop(deg);
        let mut targets = Vec::new();
        targets.try_reserve_exact(acc as usize).expect("out of memory for CSR targets");
        unsafe { targets.set_len(acc as usize) };
        self.remap = remap;
        self.ids = ids;
        self.cursor = cursor;
        self.targets = targets;
    }
    pub fn scatter(&mut self, el: &EdgeList) {
        let (remap, cur) = (&self.remap, &self.cursor);
        let tp = SyncPtr(self.targets.as_mut_ptr());
        par::for_ranges(el.len(), EDGE_GRAIN, |r| {
            for i in r {
                let (u, v) = (el.src[i], el.dst[i]);
                if u != v {
                    let (u, v) = (remap[u as usize], remap[v as usize]);
                    let a = cur[u as usize].fetch_add(1, Relaxed);
                    let b = cur[v as usize].fetch_add(1, Relaxed);
                    unsafe {
                        tp.write(a, v);
                        tp.write(b, u);
                    }
                }
            }
        });
    }
    /// End of phase 2: per-node sort + dedup, in-place compaction.
    pub fn finish(self) -> (Csr, Vec<u32>) {
        let CsrBuilder { remap, ids, cursor, mut targets, .. } = self;
        drop(remap);
        let n = ids.len();
        // after scatter cursor[v] == start of v+1; shift right to get offsets
        let mut offsets: Vec<Ix> = Vec::with_capacity(n + 1);
        offsets.push(0);
        offsets.extend(cursor[..n].iter().map(|c| c.load(Relaxed)));
        drop(cursor);
        let mut newdeg = vec![0u32; n];
        {
            let tp = SyncPtr(targets.as_mut_ptr());
            let offs = &offsets;
            par::chunks_mut(&mut newdeg, NODE_GRAIN, |ci, nd| {
                let base = ci * NODE_GRAIN;
                for (j, d) in nd.iter_mut().enumerate() {
                    let v = base + j;
                    let (s, e) = (offs[v], offs[v + 1]);
                    let sl = unsafe { std::slice::from_raw_parts_mut({ tp }.0.add(s), e - s) };
                    sl.sort_unstable();
                    let mut w = 0;
                    for r in 0..sl.len() {
                        if w == 0 || sl[r] != sl[w - 1] {
                            sl[w] = sl[r];
                            w += 1;
                        }
                    }
                    *d = w as u32;
                }
            });
        }
        let total = offsets[n];
        let newtotal: usize = newdeg.iter().map(|&d| d as usize).sum();
        if newtotal != total {
            // in-place forward compaction (new start <= old start): serial memmove
            let mut w = 0usize;
            for v in 0..n {
                let s = offsets[v];
                let d = newdeg[v] as usize;
                targets.copy_within(s..s + d, w);
                offsets[v] = w;
                w += d;
            }
            offsets[n] = w;
            targets.truncate(w);
            targets.shrink_to_fit(); // may not return memory to the OS/wasm, but allows reuse
        }
        (Csr { offsets, targets }, ids)
    }
}

/// One-shot convenience: edge list fully in memory (peak = edge list + CSR).
pub fn build_csr_undirected(el: EdgeList) -> (Csr, Vec<u32>) {
    let mut b = CsrBuilder::new();
    b.count(&el);
    b.finish_count();
    b.scatter(&el);
    drop(el);
    b.finish()
}

/// Two-pass streaming builder from text: call `push` with all chunks, then
/// `next_pass()`, push all chunks again, then `finish()`.
pub struct TextTwoPass {
    pass: u8,
    carry: Vec<u8>,
    tmp: EdgeList,
    b: CsrBuilder,
    pub edges_seen: u64,
}
impl Default for TextTwoPass {
    fn default() -> Self {
        Self::new()
    }
}
impl TextTwoPass {
    pub fn new() -> Self {
        TextTwoPass { pass: 1, carry: Vec::new(), tmp: EdgeList::default(), b: CsrBuilder::new(), edges_seen: 0 }
    }
    fn process(&mut self, buf: &[u8]) {
        self.tmp.src.clear();
        self.tmp.dst.clear();
        parse_into(buf, &mut self.tmp);
        if self.pass == 1 {
            self.edges_seen += self.tmp.len() as u64;
            self.b.count(&self.tmp);
        } else {
            self.b.scatter(&self.tmp);
        }
    }
    pub fn push(&mut self, chunk: &[u8]) {
        let mut start = 0;
        if !self.carry.is_empty() {
            match chunk.iter().position(|&b| b == b'\n') {
                Some(p) => {
                    let mut c = std::mem::take(&mut self.carry);
                    c.extend_from_slice(&chunk[..=p]);
                    self.process(&c);
                    c.clear();
                    self.carry = c;
                    start = p + 1;
                }
                None => {
                    self.carry.extend_from_slice(chunk);
                    return;
                }
            }
        }
        let rest = &chunk[start..];
        match rest.iter().rposition(|&b| b == b'\n') {
            Some(p) => {
                self.process(&rest[..=p]);
                self.carry.extend_from_slice(&rest[p + 1..]);
            }
            None => self.carry.extend_from_slice(rest),
        }
    }
    fn flush(&mut self) {
        if !self.carry.is_empty() {
            let c = std::mem::take(&mut self.carry);
            self.process(&c);
        }
    }
    pub fn next_pass(&mut self) {
        self.flush();
        self.b.finish_count();
        self.pass = 2;
    }
    pub fn finish(mut self) -> (Csr, Vec<u32>) {
        self.flush();
        drop(std::mem::take(&mut self.tmp));
        self.b.finish()
    }
}

// ---------------------------------------------------------------- PageRank

/// Pull-based PageRank on the symmetric CSR, `iters` iterations, f64.
pub fn pagerank(g: &Csr, iters: usize, damping: f64) -> Vec<f64> {
    let n = g.n();
    let nf = n as f64;
    let mut pr = vec![1.0 / nf; n];
    let mut contrib = vec![0.0f64; n];
    let mut next = vec![0.0f64; n];
    for _ in 0..iters {
        // contributions + dangling mass (per-chunk partial sums, fixed order -> deterministic)
        let prr = &pr;
        let dangl_parts = {
            let parts = std::sync::Mutex::new(vec![0.0f64; n.div_ceil(NODE_GRAIN)]);
            par::chunks_mut(&mut contrib, NODE_GRAIN, |ci, c| {
                let base = ci * NODE_GRAIN;
                let mut dsum = 0.0;
                for (j, x) in c.iter_mut().enumerate() {
                    let v = base + j;
                    let d = g.offsets[v + 1] - g.offsets[v];
                    if d == 0 {
                        dsum += prr[v];
                        *x = 0.0;
                    } else {
                        *x = prr[v] / d as f64;
                    }
                }
                parts.lock().unwrap()[ci] = dsum;
            });
            parts.into_inner().unwrap()
        };
        let dangling: f64 = dangl_parts.iter().sum();
        let base_score = (1.0 - damping) / nf + damping * dangling / nf;
        let cr = &contrib;
        par::chunks_mut(&mut next, NODE_GRAIN, |ci, out| {
            let base = ci * NODE_GRAIN;
            for (j, o) in out.iter_mut().enumerate() {
                let mut s = 0.0;
                for &u in g.nbrs(base + j) {
                    s += cr[u as usize];
                }
                *o = base_score + damping * s;
            }
        });
        std::mem::swap(&mut pr, &mut next);
    }
    pr
}

// ---------------------------------------------------------------- CC

#[inline]
fn link(u: u32, v: u32, comp: &[AtomicU32]) {
    let mut p1 = comp[u as usize].load(Relaxed);
    let mut p2 = comp[v as usize].load(Relaxed);
    while p1 != p2 {
        let (high, low) = if p1 > p2 { (p1, p2) } else { (p2, p1) };
        let p_high = comp[high as usize].load(Relaxed);
        if p_high == low {
            break;
        }
        if p_high == high
            && comp[high as usize].compare_exchange(high, low, Relaxed, Relaxed).is_ok()
        {
            break;
        }
        p1 = comp[comp[high as usize].load(Relaxed) as usize].load(Relaxed);
        p2 = comp[low as usize].load(Relaxed);
    }
}

/// Connected components by concurrent union-find (Afforest-style link, no
/// sampling). Output label = smallest node id in the component (deterministic).
pub fn connected_components(g: &Csr) -> Vec<u32> {
    let n = g.n();
    let comp: Vec<AtomicU32> = (0..n as u32).map(AtomicU32::new).collect();
    par::for_ranges(n, NODE_GRAIN, |r| {
        for v in r {
            for &u in g.nbrs(v) {
                if u < v as u32 {
                    link(v as u32, u, &comp);
                }
            }
        }
    });
    par::for_ranges(n, NODE_GRAIN, |r| {
        for v in r {
            loop {
                let c = comp[v].load(Relaxed);
                let cc = comp[c as usize].load(Relaxed);
                if c == cc {
                    break;
                }
                comp[v].store(cc, Relaxed);
            }
        }
    });
    into_plain_u32(comp)
}

pub fn cc_stats(labels: &[u32]) -> (usize, usize) {
    let mut sizes = vec![0u32; labels.len()];
    for &l in labels {
        sizes[l as usize] += 1;
    }
    let ncomp = sizes.iter().filter(|&&s| s > 0).count();
    let largest = sizes.iter().copied().max().unwrap_or(0) as usize;
    (ncomp, largest)
}

// ---------------------------------------------------------------- synthetic

/// Deterministic pseudo-random edge list (xorshift), ids < n. Used for the
/// memory-ceiling test without having to ship multi-GB text into the page.
pub fn gen_edges(n: u32, m: usize, seed: u64) -> EdgeList {
    let mut el = EdgeList::with_capacity(m);
    unsafe {
        el.src.set_len(m);
        el.dst.set_len(m);
    }
    let sp = SyncPtr(el.src.as_mut_ptr());
    let dp = SyncPtr(el.dst.as_mut_ptr());
    par::for_ranges(m, EDGE_GRAIN, |r| {
        let mut x = seed ^ ((r.start as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        for i in r {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let u = ((x >> 32) % n as u64) as u32;
            // local neighbourhood (window 4096) so the probe is cache-friendly like real graphs
            let v = ((u as u64 + 1 + (x & 0xffff_ffff) % 4096) % n as u64) as u32;
            unsafe {
                sp.write(i, u);
                dp.write(i, v);
            }
        }
    });
    el
}

/// Two-pass synthetic build: the generator is replayed (like re-reading a
/// file), so the edge list is never resident. Batches of 4M edges.
pub fn gen_csr_two_pass(n: u32, m: usize, seed: u64) -> (Csr, Vec<u32>) {
    const B: usize = 1 << 22;
    let mut b = CsrBuilder::new();
    for pass in 0..2 {
        let mut done = 0;
        let mut k = 0u64;
        while done < m {
            let len = B.min(m - done);
            let el = gen_edges(n, len, seed.wrapping_add(k.wrapping_mul(0x2545_F491_4F6C_DD1D)));
            if pass == 0 { b.count(&el) } else { b.scatter(&el) }
            done += len;
            k += 1;
        }
        if pass == 0 {
            b.finish_count();
        }
    }
    b.finish()
}

pub fn pr_checksum(pr: &[f64]) -> (f64, f64) {
    let s: f64 = pr.iter().sum();
    let mx = pr.iter().copied().fold(0.0, f64::max);
    (s, mx)
}
