//! Q1: ID dictionary (external string id <-> dense u32).
//! usage: dict --method {hash,mphf,fst,sorted,fc} --kind {eth,ethbin,user} --n N --dir WORK [--threads T] [--queries Q]
use f1q::*;
use rayon::prelude::*;
use serde_json::json;
use std::path::{Path, PathBuf};

const FIRST: &[&str] = &[
    "james", "mary", "john", "patricia", "robert", "jennifer", "michael", "linda", "william", "elizabeth", "david",
    "barbara", "richard", "susan", "joseph", "jessica", "thomas", "sarah", "charles", "karen", "chris", "nancy",
    "daniel", "lisa", "matthew", "betty", "anthony", "margaret", "mark", "sandra", "marco", "giulia", "luca",
    "francesca", "ahmed", "fatima", "wei", "li", "yuki", "hiro", "olga", "ivan", "ana", "jose", "maria", "carlos",
    "sofia", "pierre", "marie", "hans", "anna", "lars", "emma", "noah", "liam", "olivia", "ava", "mia", "ethan",
    "lucas", "leo", "zoe", "max", "lena",
];
const LAST: &[&str] = &[
    "smith", "johnson", "williams", "brown", "jones", "garcia", "miller", "davis", "rodriguez", "martinez",
    "hernandez", "lopez", "gonzalez", "wilson", "anderson", "thomas", "taylor", "moore", "jackson", "martin", "lee",
    "perez", "thompson", "white", "harris", "sanchez", "clark", "ramirez", "lewis", "robinson", "rossi", "russo",
    "ferrari", "esposito", "bianchi", "romano", "colombo", "muller", "schmidt", "schneider", "fischer", "weber",
    "meyer", "wagner", "becker", "dubois", "durand", "leroy", "moreau", "ivanov", "smirnov", "kuznetsov", "wang",
    "zhang", "liu", "chen", "yang", "huang", "zhao", "wu", "zhou", "kim", "park", "nguyen",
];
const DOMAINS: &[(&str, u32)] = &[
    ("https://twitter.com/", 30),
    ("https://www.instagram.com/", 20),
    ("https://github.com/", 10),
    ("https://www.facebook.com/profile/", 15),
    ("https://www.linkedin.com/in/", 10),
    ("https://t.me/", 5),
    ("https://mastodon.social/@", 5),
    ("https://www.reddit.com/user/", 5),
];
const SEPS: &[&str] = &["", ".", "_", "-"];

fn gen_key(kind: &str, i: u64, out: &mut Vec<u8>) {
    let h0 = splitmix(i.wrapping_mul(0x2545F4914F6CDD1D) ^ 0xABCDEF);
    match kind {
        "eth" | "ethbin" => {
            let mut b = [0u8; 24];
            for j in 0..3 {
                b[j * 8..j * 8 + 8].copy_from_slice(&splitmix(h0 ^ (j as u64 + 1) * 0x9E37).to_le_bytes());
            }
            if kind == "ethbin" {
                out.extend_from_slice(&b[..20]);
            } else {
                out.extend_from_slice(b"0x");
                const HEX: &[u8] = b"0123456789abcdef";
                for &x in &b[..20] {
                    out.push(HEX[(x >> 4) as usize]);
                    out.push(HEX[(x & 15) as usize]);
                }
            }
        }
        "user" => {
            let mut r = Rng(h0);
            let mut w = r.below(100) as u32;
            let mut d = DOMAINS[0].0;
            for &(dom, wt) in DOMAINS {
                if w < wt {
                    d = dom;
                    break;
                }
                w -= wt;
            }
            out.extend_from_slice(d.as_bytes());
            out.extend_from_slice(FIRST[r.below(FIRST.len() as u64) as usize].as_bytes());
            out.extend_from_slice(SEPS[r.below(4) as usize].as_bytes());
            out.extend_from_slice(LAST[r.below(LAST.len() as u64) as usize].as_bytes());
            // unique suffix: the index itself (realistic "name + number" handles)
            out.extend_from_slice(i.to_string().as_bytes());
        }
        _ => panic!("kind"),
    }
}

pub struct Pool {
    pub data: Vec<u8>,
    pub off: Vec<u64>,
}
impl Pool {
    #[inline]
    pub fn key(&self, i: usize) -> &[u8] {
        &self.data[self.off[i] as usize..self.off[i + 1] as usize]
    }
    pub fn len(&self) -> usize {
        self.off.len() - 1
    }
}

fn gen_pool(kind: &str, n: usize) -> Pool {
    let fixed = match kind {
        "eth" => 42,
        "ethbin" => 20,
        _ => 0,
    };
    if fixed > 0 {
        // fixed-length keys: generate in place (no 2x peak at 500M keys)
        let mut data = vec![0u8; n * fixed];
        data.par_chunks_mut(fixed).enumerate().for_each_init(
            || Vec::with_capacity(64),
            |b, (i, d)| {
                b.clear();
                gen_key(kind, i as u64, b);
                d.copy_from_slice(b);
            },
        );
        let off: Vec<u64> = (0..=n as u64).into_par_iter().map(|i| i * fixed as u64).collect();
        return Pool { data, off };
    }
    let ids: Vec<u64> = (0..n as u64).collect();
    let chunks: Vec<(Vec<u8>, Vec<u32>)> = ids
        .par_chunks(1 << 18)
        .map(|c| {
            let mut d = Vec::with_capacity(c.len() * 44);
            let mut l = Vec::with_capacity(c.len());
            for &i in c {
                let s = d.len();
                gen_key(kind, i, &mut d);
                l.push((d.len() - s) as u32);
            }
            (d, l)
        })
        .collect();
    let total: usize = chunks.iter().map(|c| c.0.len()).sum();
    let mut data = Vec::with_capacity(total);
    let mut off = Vec::with_capacity(ids.len() + 1);
    off.push(0u64);
    for (d, l) in chunks {
        data.extend_from_slice(&d);
        for x in l {
            let last = *off.last().unwrap();
            off.push(last + x as u64);
        }
    }
    Pool { data, off }
}

// ---------- mmapped string pool (reverse lookup id -> string) ----------
struct MPool {
    data: MFile,
    off: MFile,
}
impl MPool {
    fn open(dir: &Path, name: &str) -> MPool {
        MPool {
            data: MFile::open(dir.join(format!("{name}.bin")), true),
            off: MFile::open(dir.join(format!("{name}.off.u64")), true),
        }
    }
    #[inline]
    fn key(&self, i: usize) -> &[u8] {
        let o = self.off.as_slice::<u64>();
        &self.data.bytes()[o[i] as usize..o[i + 1] as usize]
    }
    fn evict(&self) {
        self.data.evict();
        self.off.evict();
    }
    fn bytes(&self) -> u64 {
        (self.data.map.len() + self.off.map.len()) as u64
    }
}
fn write_pool(dir: &Path, name: &str, p: &Pool) {
    write_slice(dir.join(format!("{name}.bin")), &p.data);
    write_slice(dir.join(format!("{name}.off.u64")), &p.off);
}

trait Dict: Sync {
    fn get(&self, k: &[u8]) -> Option<u32>;
    fn rev(&self, id: u32, out: &mut Vec<u8>);
    fn evict(&self) {}
    fn files(&self) -> Vec<&MFile> {
        vec![]
    }
    /// ingest-path lookup when the key is known to be in the set: (no verification, id = slot2id[h]) and (id = h)
    fn noverify(&self, _k: &[u8]) -> Option<(u64, u64)> {
        None
    }
    fn prefix(&self, _p: &[u8], _limit: usize) -> Option<usize> {
        None
    }
}

// ---------- (d) hashbrown in RAM ----------
struct HashDict<'a> {
    h: hashbrown::HashMap<&'a [u8], u32, rustc_hash::FxBuildHasher>,
    pool: &'a Pool,
}
impl<'a> Dict for HashDict<'a> {
    fn get(&self, k: &[u8]) -> Option<u32> {
        self.h.get(k).copied()
    }
    fn rev(&self, id: u32, out: &mut Vec<u8>) {
        out.clear();
        out.extend_from_slice(self.pool.key(id as usize));
    }
}

// ---------- (b) MPHF + slot2id + verification against pool ----------
type Mph = ptr_hash::PtrHash<
    [u8],
    ptr_hash::bucket_fn::CubicEps,
    Vec<u32>,
    ptr_hash::hash::Xxh3_128,
    Vec<u8>,
    false,
    true,
>;
struct MphDict {
    mph: Mph,
    slot2id: MFile,
    pool: MPool,
}
impl Dict for MphDict {
    #[inline]
    fn noverify(&self, k: &[u8]) -> Option<(u64, u64)> {
        let h = self.mph.index(k);
        Some((self.slot2id.as_slice::<u32>()[h] as u64, h as u64))
    }
    fn files(&self) -> Vec<&MFile> {
        vec![&self.slot2id, &self.pool.data, &self.pool.off]
    }
    fn get(&self, k: &[u8]) -> Option<u32> {
        let s = self.mph.index(k);
        let id = self.slot2id.as_slice::<u32>()[s];
        if self.pool.key(id as usize) == k { Some(id) } else { None }
    }
    fn rev(&self, id: u32, out: &mut Vec<u8>) {
        out.clear();
        out.extend_from_slice(self.pool.key(id as usize));
    }
    fn evict(&self) {
        self.slot2id.evict();
        self.pool.evict();
    }
}

// ---------- (a) FST map + pool for reverse ----------
struct FstDict {
    file: MFile,
    pool: MPool,
}
impl FstDict {
    fn map(&self) -> fst::Map<&[u8]> {
        fst::Map::new(self.file.bytes()).unwrap()
    }
}
impl Dict for FstDict {
    fn files(&self) -> Vec<&MFile> {
        vec![&self.file, &self.pool.data, &self.pool.off]
    }
    fn get(&self, k: &[u8]) -> Option<u32> {
        self.map().get(k).map(|v| v as u32)
    }
    fn rev(&self, id: u32, out: &mut Vec<u8>) {
        out.clear();
        out.extend_from_slice(self.pool.key(id as usize));
    }
    fn evict(&self) {
        self.file.evict();
        self.pool.evict();
    }
    fn prefix(&self, p: &[u8], limit: usize) -> Option<usize> {
        use fst::{IntoStreamer, Streamer};
        let m = self.map();
        let mut st = m.range().ge(p).into_stream();
        let mut c = 0;
        while let Some((k, _)) = st.next() {
            if !k.starts_with(p) || c >= limit {
                break;
            }
            c += 1;
        }
        Some(c)
    }
}

// ---------- (c0) sorted pool + binary search; (c) front-coded blocks ----------
struct SortedDict {
    spool: MPool,
    rank2id: MFile,
    id2rank: MFile,
}
impl SortedDict {
    fn lower_bound(&self, k: &[u8]) -> usize {
        let n = self.spool.off.as_slice::<u64>().len() - 1;
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.spool.key(mid) < k { lo = mid + 1 } else { hi = mid }
        }
        lo
    }
}
impl Dict for SortedDict {
    fn files(&self) -> Vec<&MFile> {
        vec![&self.spool.data, &self.spool.off, &self.rank2id, &self.id2rank]
    }
    fn get(&self, k: &[u8]) -> Option<u32> {
        let r = self.lower_bound(k);
        let n = self.spool.off.as_slice::<u64>().len() - 1;
        if r < n && self.spool.key(r) == k { Some(self.rank2id.as_slice::<u32>()[r]) } else { None }
    }
    fn rev(&self, id: u32, out: &mut Vec<u8>) {
        let r = self.id2rank.as_slice::<u32>()[id as usize];
        out.clear();
        out.extend_from_slice(self.spool.key(r as usize));
    }
    fn evict(&self) {
        self.spool.evict();
        self.rank2id.evict();
        self.id2rank.evict();
    }
    fn prefix(&self, p: &[u8], limit: usize) -> Option<usize> {
        let n = self.spool.off.as_slice::<u64>().len() - 1;
        let mut r = self.lower_bound(p);
        let mut c = 0;
        while r < n && c < limit && self.spool.key(r).starts_with(p) {
            c += 1;
            r += 1;
        }
        Some(c)
    }
}

const FCB: usize = 16;
#[inline]
fn put_var(out: &mut Vec<u8>, mut x: u64) {
    while x >= 128 {
        out.push((x as u8) | 128);
        x >>= 7;
    }
    out.push(x as u8);
}
#[inline]
fn get_var(b: &[u8], p: &mut usize) -> usize {
    let mut x = 0usize;
    let mut s = 0;
    loop {
        let c = b[*p];
        *p += 1;
        x |= ((c & 127) as usize) << s;
        if c < 128 {
            return x;
        }
        s += 7;
    }
}
struct FcDict {
    n: usize,
    data: MFile,
    blk: MFile,
    rank2id: MFile,
    id2rank: MFile,
}
impl FcDict {
    #[inline]
    fn head(&self, b: usize) -> &[u8] {
        let d = self.data.bytes();
        let mut p = self.blk.as_slice::<u64>()[b] as usize;
        let l = get_var(d, &mut p);
        &d[p..p + l]
    }
    /// decode block b, calling f(rank, key) until it returns false
    fn scan(&self, b: usize, buf: &mut Vec<u8>, mut f: impl FnMut(usize, &[u8]) -> bool) {
        let d = self.data.bytes();
        let mut p = self.blk.as_slice::<u64>()[b] as usize;
        let l = get_var(d, &mut p);
        buf.clear();
        buf.extend_from_slice(&d[p..p + l]);
        p += l;
        let end = ((b + 1) * FCB).min(self.n);
        let mut r = b * FCB;
        loop {
            if !f(r, buf) {
                return;
            }
            r += 1;
            if r >= end {
                return;
            }
            let lcp = get_var(d, &mut p);
            let sl = get_var(d, &mut p);
            buf.truncate(lcp);
            buf.extend_from_slice(&d[p..p + sl]);
            p += sl;
        }
    }
    /// first block whose head > k, minus one
    fn find_block(&self, k: &[u8]) -> usize {
        let nb = self.blk.as_slice::<u64>().len();
        let (mut lo, mut hi) = (0usize, nb);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.head(mid) <= k { lo = mid + 1 } else { hi = mid }
        }
        lo.saturating_sub(1)
    }
}
impl Dict for FcDict {
    fn files(&self) -> Vec<&MFile> {
        vec![&self.data, &self.blk, &self.rank2id, &self.id2rank]
    }
    fn get(&self, k: &[u8]) -> Option<u32> {
        let b = self.find_block(k);
        let mut buf = Vec::with_capacity(64);
        let mut res = None;
        self.scan(b, &mut buf, |r, key| {
            if key == k {
                res = Some(r);
                false
            } else {
                key < k
            }
        });
        res.map(|r| self.rank2id.as_slice::<u32>()[r])
    }
    fn rev(&self, id: u32, out: &mut Vec<u8>) {
        let r = self.id2rank.as_slice::<u32>()[id as usize] as usize;
        let mut buf = Vec::with_capacity(64);
        self.scan(r / FCB, &mut buf, |rr, key| {
            if rr == r {
                out.clear();
                out.extend_from_slice(key);
                false
            } else {
                true
            }
        });
    }
    fn evict(&self) {
        self.data.evict();
        self.blk.evict();
        self.rank2id.evict();
        self.id2rank.evict();
    }
    fn prefix(&self, p: &[u8], limit: usize) -> Option<usize> {
        let nb = self.blk.as_slice::<u64>().len();
        let mut b = self.find_block(p);
        let mut c = 0;
        let mut buf = Vec::with_capacity(64);
        let mut done = false;
        while b < nb && !done {
            self.scan(b, &mut buf, |_, key| {
                if key < p {
                    return true;
                }
                if key.starts_with(p) && c < limit {
                    c += 1;
                    true
                } else {
                    done = true;
                    false
                }
            });
            b += 1;
        }
        Some(c)
    }
}

fn sort_perm(pool: &Pool) -> Vec<u32> {
    let mut perm: Vec<u32> = (0..pool.len() as u32).collect();
    perm.par_sort_unstable_by(|&a, &b| pool.key(a as usize).cmp(pool.key(b as usize)));
    perm
}
fn write_ranks(dir: &Path, perm: &[u32]) {
    let mut id2rank = vec![0u32; perm.len()];
    for (r, &id) in perm.iter().enumerate() {
        id2rank[id as usize] = r as u32;
    }
    write_slice(dir.join("rank2id.u32"), perm);
    write_slice(dir.join("id2rank.u32"), &id2rank);
}

struct SyncPtr(*mut u32);
unsafe impl Sync for SyncPtr {}
unsafe impl Send for SyncPtr {}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let method = arg_s(&args, "method", "hash");
    let kind = arg_s(&args, "kind", "eth");
    let n: usize = arg(&args, "n", 4_000_000);
    let threads: usize = arg(&args, "threads", rayon::current_num_threads());
    let nq: usize = arg(&args, "queries", 5_000_000);
    let ncold: usize = arg(&args, "cold", 300);
    let work = PathBuf::from(arg_s(&args, "dir", "work"));
    let out = arg_s(&args, "out", "");
    let dir = work.join(format!("dict_{kind}_{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    let build_threads = rayon::current_num_threads();
    eprintln!("method={method} kind={kind} n={n} build_threads={build_threads} query_threads={threads}");

    // keys in "ingest order": dense id i is the i-th distinct key seen
    let t = now();
    let pool = gen_pool(&kind, n);
    let gen_s = secs(t);
    let key_bytes = pool.data.len();
    eprintln!("gen {:.2}s, {} bytes keys (avg {:.1} B)", gen_s, key_bytes, key_bytes as f64 / n as f64);
    if !dir.join("pool_id.bin").exists() {
        write_pool(&dir, "pool_id", &pool);
    }
    // query keys: random existing keys, copied out
    let mut r = Rng(42);
    let qidx: Vec<usize> = (0..nq).map(|_| r.below(n as u64) as usize).collect();
    let qpool = {
        let mut data = Vec::new();
        let mut off = vec![0u64];
        for &i in &qidx {
            data.extend_from_slice(pool.key(i));
            off.push(data.len() as u64);
        }
        Pool { data, off }
    };
    let pfx: Vec<Vec<u8>> = (0..200)
        .map(|j| {
            let k = pool.key(qidx[j]);
            let l = if kind == "user" { k.len().saturating_sub(8).max(1) } else { 6.min(k.len()) };
            k[..l].to_vec()
        })
        .collect();

    if method == "dedupe" {
        // ingest pass 1: an endpoint stream of n keys drawn with replacement -> distinct keys + dense ids,
        // parallel: hash-partition into P shards, then each shard dedupes with its own hash map.
        let mut r = Rng(7);
        let sidx: Vec<u32> = (0..n).map(|_| r.below(n as u64) as u32).collect();
        let tp = f1q::pool(threads);
        let rss0 = rss_mb();
        reset_hwm();
        let t = now();
        const P: usize = 256;
        let (ids, distinct) = tp.install(|| {
            use std::hash::BuildHasher;
            let hb = rustc_hash::FxBuildHasher;
            // phase 1: per-chunk partition
            let parts: Vec<Vec<Vec<u32>>> = (0..n)
                .into_par_iter()
                .chunks(1 << 20)
                .map(|ch| {
                    let mut p: Vec<Vec<u32>> = vec![Vec::new(); P];
                    for j in ch {
                        let k = pool.key(sidx[j] as usize);
                        p[(hb.hash_one(k) >> 56) as usize].push(j as u32);
                    }
                    p
                })
                .collect();
            // phase 2: per shard dedupe; local ids
            let shard: Vec<(Vec<(u32, u32)>, u32)> = (0..P)
                .into_par_iter()
                .map(|s| {
                    let mut h: hashbrown::HashMap<&[u8], u32, rustc_hash::FxBuildHasher> = Default::default();
                    let mut out = Vec::new();
                    for c in &parts {
                        for &j in &c[s] {
                            let k = pool.key(sidx[j as usize] as usize);
                            let l = h.len() as u32;
                            let id = *h.entry(k).or_insert(l);
                            out.push((j, id));
                        }
                    }
                    (out, h.len() as u32)
                })
                .collect();
            let mut base = vec![0u32; P + 1];
            for s in 0..P {
                base[s + 1] = base[s] + shard[s].1;
            }
            let mut ids = vec![0u32; n];
            let sp = SyncPtr(ids.as_mut_ptr());
            shard.par_iter().enumerate().for_each(|(s, (v, _))| {
                let _ = &sp;
                for &(j, id) in v {
                    unsafe { *sp.0.add(j as usize) = base[s] + id };
                }
            });
            (ids, base[P])
        });
        let el = secs(t);
        let res = json!({"method": "dedupe", "kind": kind, "n_stream": n, "distinct": distinct, "threads": threads,
            "s": el, "mkeys_per_s": n as f64 / el / 1e6, "peak_extra_rss_mb": hwm_mb() - rss0,
            "note": "pass-1 of ingest: hash-partitioned (256 shards) parallel dedupe + dense id assignment, all in RAM"});
        assert!(ids.iter().all(|&x| x < distinct));
        println!("{}", serde_json::to_string(&res).unwrap());
        if !out.is_empty() {
            save_json(&out, &res);
        }
        return;
    }
    let rss0 = rss_mb();
    reset_hwm();
    let mut build = json!({});
    let mut disk_files: Vec<PathBuf> = vec![];
    let t = now();
    let dict: Box<dyn Dict + '_> = match method.as_str() {
        "hash" => {
            let mut h = hashbrown::HashMap::with_capacity_and_hasher(n, rustc_hash::FxBuildHasher);
            for i in 0..n {
                h.insert(pool.key(i), i as u32);
            }
            build["build_s"] = json!(secs(t));
            build["threads"] = json!(1);
            Box::new(HashDict { h, pool: &pool })
        }
        "mphf" => {
            let mph: Mph = Mph::new_from_par_iter(
                n,
                (0..n).into_par_iter().map(|i| pool.key(i)),
                ptr_hash::PtrHashParams::default_balanced(),
            );
            let t_mph = secs(t);
            let mut slot2id = vec![0u32; n];
            let sp = SyncPtr(slot2id.as_mut_ptr());
            (0..n).into_par_iter().for_each(|i| {
                let s = mph.index(pool.key(i));
                let _ = &sp;
                unsafe { *sp.0.add(s) = i as u32 };
            });
            write_slice(dir.join("slot2id.u32"), &slot2id);
            drop(slot2id);
            let mph_bytes = mem_dbg::MemSize::mem_size(&mph, mem_dbg::SizeFlags::default());
            build["build_s"] = json!(secs(t));
            build["mphf_build_s"] = json!(t_mph);
            build["mphf_bytes"] = json!(mph_bytes);
            build["mphf_bits_per_key"] = json!(mph_bytes as f64 * 8.0 / n as f64);
            build["threads"] = json!(build_threads);
            disk_files.push(dir.join("slot2id.u32"));
            disk_files.push(dir.join("pool_id.bin"));
            disk_files.push(dir.join("pool_id.off.u64"));
            build["extra_disk_bytes"] = json!(mph_bytes);
            Box::new(MphDict {
                mph,
                slot2id: MFile::open(dir.join("slot2id.u32"), true),
                pool: MPool::open(&dir, "pool_id"),
            })
        }
        "fst" => {
            let perm = sort_perm(&pool);
            let sort_s = secs(t);
            let t2 = now();
            let f = std::io::BufWriter::with_capacity(1 << 22, std::fs::File::create(dir.join("fst.map")).unwrap());
            let mut b = fst::MapBuilder::new(f).unwrap();
            for &id in &perm {
                b.insert(pool.key(id as usize), id as u64).unwrap();
            }
            b.finish().unwrap();
            let fst_s = secs(t2);
            // range-sharded parallel FST build (one FST per contiguous key range), for scaling reference
            let t3 = now();
            let p = build_threads.max(1);
            let per = n.div_ceil(p);
            let shard_bytes: u64 = perm
                .par_chunks(per)
                .enumerate()
                .map(|(si, ch)| {
                    let path = dir.join(format!("fst_shard_{si}.map"));
                    let f = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&path).unwrap());
                    let mut b = fst::MapBuilder::new(f).unwrap();
                    for &id in ch {
                        b.insert(pool.key(id as usize), id as u64).unwrap();
                    }
                    b.finish().unwrap();
                    let sz = file_size(&path);
                    std::fs::remove_file(&path).ok();
                    sz
                })
                .sum();
            build["fst_sharded_build_s"] = json!(secs(t3));
            build["fst_shards"] = json!(p);
            build["fst_sharded_total_bytes"] = json!(shard_bytes);
            build["fst_build_s_single"] = json!(fst_s);
            build["sort_s"] = json!(sort_s);
            build["fst_build_s"] = json!(fst_s);
            build["build_s"] = json!(sort_s + fst_s);
            build["threads"] = json!(format!("sort {build_threads}, fst 1"));
            disk_files.push(dir.join("fst.map"));
            disk_files.push(dir.join("pool_id.bin"));
            disk_files.push(dir.join("pool_id.off.u64"));
            Box::new(FstDict { file: MFile::open(dir.join("fst.map"), true), pool: MPool::open(&dir, "pool_id") })
        }
        "sorted" => {
            let perm = sort_perm(&pool);
            let sort_s = secs(t);
            let sp = {
                let mut data = Vec::with_capacity(pool.data.len());
                let mut off = Vec::with_capacity(n + 1);
                off.push(0);
                for &id in &perm {
                    data.extend_from_slice(pool.key(id as usize));
                    off.push(data.len() as u64);
                }
                Pool { data, off }
            };
            write_pool(&dir, "spool", &sp);
            drop(sp);
            write_ranks(&dir, &perm);
            build["sort_s"] = json!(sort_s);
            build["build_s"] = json!(secs(t));
            build["threads"] = json!(format!("sort {build_threads}, write 1"));
            for f in ["spool.bin", "spool.off.u64", "rank2id.u32", "id2rank.u32"] {
                disk_files.push(dir.join(f));
            }
            Box::new(SortedDict {
                spool: MPool::open(&dir, "spool"),
                rank2id: MFile::open(dir.join("rank2id.u32"), true),
                id2rank: MFile::open(dir.join("id2rank.u32"), true),
            })
        }
        "fc" => {
            let perm = sort_perm(&pool);
            let sort_s = secs(t);
            let mut data = Vec::with_capacity(pool.data.len() / 2);
            let mut blk = Vec::with_capacity(n / FCB + 1);
            let mut prev: &[u8] = &[];
            for (r, &id) in perm.iter().enumerate() {
                let k = pool.key(id as usize);
                if r % FCB == 0 {
                    blk.push(data.len() as u64);
                    put_var(&mut data, k.len() as u64);
                    data.extend_from_slice(k);
                } else {
                    let lcp = k.iter().zip(prev).take_while(|(a, b)| a == b).count();
                    put_var(&mut data, lcp as u64);
                    put_var(&mut data, (k.len() - lcp) as u64);
                    data.extend_from_slice(&k[lcp..]);
                }
                prev = k;
            }
            write_slice(dir.join("fc.bin"), &data);
            write_slice(dir.join("fc.blk.u64"), &blk);
            drop(data);
            write_ranks(&dir, &perm);
            build["sort_s"] = json!(sort_s);
            build["build_s"] = json!(secs(t));
            build["threads"] = json!(format!("sort {build_threads}, encode 1"));
            for f in ["fc.bin", "fc.blk.u64", "rank2id.u32", "id2rank.u32"] {
                disk_files.push(dir.join(f));
            }
            Box::new(FcDict {
                n,
                data: MFile::open(dir.join("fc.bin"), true),
                blk: MFile::open(dir.join("fc.blk.u64"), true),
                rank2id: MFile::open(dir.join("rank2id.u32"), true),
                id2rank: MFile::open(dir.join("id2rank.u32"), true),
            })
        }
        _ => panic!("method"),
    };
    build["peak_extra_rss_mb"] = json!(hwm_mb() - rss0);
    build["rss_after_mb_minus_keys"] = json!(rss_mb() - rss0);
    let mut disk: u64 = disk_files.iter().map(file_size).sum();
    disk += build["extra_disk_bytes"].as_u64().unwrap_or(0);
    let mut files = json!({});
    for f in &disk_files {
        files[f.file_name().unwrap().to_str().unwrap()] = json!(file_size(f));
    }
    eprintln!("build {}", build);

    // correctness spot check
    for j in 0..1000 {
        assert_eq!(dict.get(qpool.key(j)), Some(qidx[j] as u32), "lookup mismatch");
    }
    let mut miss = b"0xnotakey".to_vec();
    miss.extend_from_slice(&qpool.key(0)[9.min(qpool.key(0).len())..]);
    assert_eq!(dict.get(&miss), None);

    // warm latency (single thread): make every file resident first
    let mut sink = 0u64;
    for f in dict.files() {
        sink += f.warm();
    }
    let resident: Vec<f64> = dict.files().iter().map(|f| f.resident()).collect();
    eprintln!("resident after warm: {:?}", resident);
    for j in 0..nq.min(2_000_000) {
        sink += dict.get(qpool.key(j)).unwrap() as u64; // warm-up pass
    }
    let nw = 200_000.min(nq);
    let mut lat = Vec::with_capacity(nw);
    for j in 0..nw {
        let t = now();
        sink += dict.get(qpool.key(j)).unwrap() as u64;
        lat.push(t.elapsed().as_nanos() as f64 / 1000.0);
    }
    let warm = lat_summary(&mut lat);
    let t = now();
    for j in 0..nw {
        sink += dict.get(qpool.key(j)).unwrap() as u64;
    }
    let warm_loop_ns = secs(t) * 1e9 / nw as f64;
    // reverse warm
    let mut buf = Vec::new();
    let mut lat = Vec::with_capacity(nw);
    for j in 0..nw {
        let t = now();
        dict.rev(qidx[j] as u32, &mut buf);
        lat.push(t.elapsed().as_nanos() as f64 / 1000.0);
        sink += buf.len() as u64;
    }
    dict.rev(qidx[7] as u32, &mut buf);
    assert_eq!(&buf[..], qpool.key(7));
    let rev_warm = lat_summary(&mut lat);

    // batch throughput
    let tp = f1q::pool(threads);
    let t = now();
    let s: u64 = tp.install(|| {
        (0..nq).into_par_iter().with_min_len(4096).map(|j| dict.get(qpool.key(j)).unwrap() as u64).sum()
    });
    let batch_s = secs(t);
    sink += s;
    let batch_mops = nq as f64 / batch_s / 1e6;
    let mut ingest = serde_json::Value::Null;
    if dict.noverify(qpool.key(0)).is_some() {
        let t = now();
        let s: u64 = tp.install(|| {
            (0..nq).into_par_iter().with_min_len(4096).map(|j| dict.noverify(qpool.key(j)).unwrap().0).sum()
        });
        let a = nq as f64 / secs(t) / 1e6;
        let t = now();
        let s2: u64 = tp.install(|| {
            (0..nq).into_par_iter().with_min_len(4096).map(|j| dict.noverify(qpool.key(j)).unwrap().1).sum()
        });
        let b = nq as f64 / secs(t) / 1e6;
        sink += s + s2;
        ingest = json!({"threads": threads, "mlookups_per_s_noverify_slot2id": a, "mlookups_per_s_id_is_slot": b});
    }

    // prefix
    let mut pre = serde_json::Value::Null;
    if dict.prefix(&pfx[0], 10).is_some() {
        let mut lat = Vec::new();
        let mut tot = 0;
        for p in &pfx {
            let t = now();
            tot += dict.prefix(p, 100).unwrap();
            lat.push(t.elapsed().as_nanos() as f64 / 1000.0);
        }
        pre = json!({"limit": 100, "avg_hits": tot as f64 / pfx.len() as f64, "lat": lat_summary(&mut lat)});
    }

    // cold
    let mut cold = serde_json::Value::Null;
    let mut rev_cold = serde_json::Value::Null;
    let mut semicold = serde_json::Value::Null;
    if method != "hash" {
        let mut lat = Vec::new();
        let mut latr = Vec::new();
        let mut max_res = 0f64;
        for j in 0..ncold {
            dict.evict();
            if j % 50 == 0 {
                for f in dict.files() {
                    max_res = max_res.max(f.resident());
                }
            }
            let k = qpool.key(nw + j);
            let t = now();
            sink += dict.get(k).unwrap() as u64;
            lat.push(t.elapsed().as_nanos() as f64 / 1000.0);
            dict.evict();
            let t = now();
            dict.rev(qidx[nq - 1 - j] as u32, &mut buf); // different id than the forward probe (NVMe-internal cache)
            latr.push(t.elapsed().as_nanos() as f64 / 1000.0);
        }
        cold = lat_summary(&mut lat);
        cold["max_resident_fraction_after_evict"] = json!(max_res);
        rev_cold = lat_summary(&mut latr);
        // semi-cold: evict once, then many lookups; upper index levels become cached,
        // leaves mostly not (approximates a dictionary much larger than RAM)
        dict.evict();
        let ns = 6000.min(nq - nw - ncold);
        let mut lat = Vec::new();
        for j in 0..ns {
            let k = qpool.key(nw + ncold + j);
            let t = now();
            sink += dict.get(k).unwrap() as u64;
            if j >= ns - 2000 {
                lat.push(t.elapsed().as_nanos() as f64 / 1000.0);
            }
        }
        semicold = lat_summary(&mut lat);
    }
    eprintln!("sink {sink}");
    let res = json!({
        "method": method, "kind": kind, "n": n, "avg_key_bytes": key_bytes as f64 / n as f64,
        "gen_s": gen_s, "build": build, "disk_bytes": disk, "disk_bytes_per_key": disk as f64 / n as f64,
        "files": files,
        "fwd_warm": warm, "fwd_warm_loop_ns": warm_loop_ns, "rev_warm": rev_warm,
        "batch": {"threads": threads, "queries": nq, "s": batch_s, "mlookups_per_s": batch_mops,
                  "extrapolated_s_for_3.6e9_endpoint_lookups": 3.6e9 / (batch_mops * 1e6)},
        "ingest_map": ingest, "prefix": pre, "fwd_cold": cold, "rev_cold": rev_cold, "fwd_semicold_last2000_of_6000": semicold,
        "resident_after_warm": resident, "note": "latency per op incl ~20-30ns Instant overhead; cold = madvise+fadvise DONTNEED before each op, MADV_RANDOM",
    });
    println!("{}", serde_json::to_string(&res).unwrap());
    if !out.is_empty() {
        save_json(&out, &res);
    }
}
