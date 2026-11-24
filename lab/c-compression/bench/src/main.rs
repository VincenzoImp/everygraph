//! glc — topology compression lab tool (experiment C).
//! Subcommands: bv2csr, sym, csr2bv, encode, bench, perm, order, idwidth.
mod codecs;
mod csr;
mod svb;

use anyhow::{Context, Result, bail};
use codecs::*;
use csr::*;
use rayon::prelude::*;
use serde_json::json;
use dsi_bitstream::traits::BE;
use lender::Lender;
use std::path::Path;
use std::time::Instant;
use webgraph::prelude::*;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        bail!("usage: glc <cmd> ...");
    }
    let a = &args[2..];
    match args[1].as_str() {
        "bv2csr" => bv2csr(&a[0], &a[1]),
        "sym" => symmetrize(&a[0], &a[1]),
        "csr2bv" => csr2bv(&a[0], &a[1], a[2].parse()?, a[3].parse()?),
        "encode" => encode_cmd(&a[0], &a[1], &a[2]),
        "bench" => bench_cmd(a),
        "perm" => perm_cmd(&a[0], &a[1], &a[2]),
        "order" => order_cmd(&a[0], &a[1], &a[2]),
        "idwidth" => idwidth_cmd(&a[0]),
        "readbw" => readbw(&a[0], a[1].parse()?),
        "checksym" => checksym(&a[0], &a[1]),
        _ => bail!("unknown cmd"),
    }
}

// ---------------------------------------------------------------- bv2csr
fn bv2csr(basename: &str, outdir: &str) -> Result<()> {
    let t0 = Instant::now();
    let tmp = format!("{outdir}.tmp");
    std::fs::create_dir_all(&tmp)?;
    let g = BvGraph::with_basename(basename).endianness::<BE>().load()?;
    let n = g.num_nodes();
    let seq = BvGraphSeq::with_basename(basename).endianness::<BE>().load()?;
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0u64);
    let mut it = seq.offset_deg_iter();
    let mut acc = 0u64;
    for _ in 0..n {
        acc += it.next_degree()? as u64;
        offsets.push(acc);
    }
    let m = acc as usize;
    eprintln!("degrees done n={n} m={m} {:?}", t0.elapsed());
    write_u64s(&format!("{tmp}/offsets.u64"), &offsets)?;
    let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(format!("{tmp}/targets.u32"))?;
    f.set_len((m * 4) as u64)?;
    let mut mm = unsafe { memmap2::MmapMut::map_mut(&f)? };
    let tg: &mut [u32] = unsafe { std::slice::from_raw_parts_mut(mm.as_mut_ptr() as *mut u32, m) };
    let chunks = make_chunks(&offsets, 4096);
    let tp = SendPtr(tg.as_mut_ptr());
    chunks.par_iter().for_each(|&(lo, hi)| {
        let tp = &tp;
        let mut l = g.iter_from(lo);
        let mut pos = offsets[lo] as usize;
        for _ in lo..hi {
            let (_, succ) = l.next().unwrap();
            for s in succ {
                unsafe { *tp.0.add(pos) = s as u32 };
                pos += 1;
            }
        }
        assert_eq!(pos as u64, offsets[hi]);
    });
    mm.flush()?;
    drop(mm);
    eprintln!("targets done {:?}", t0.elapsed());
    let ids: Vec<u64> = (0..n as u64).collect();
    write_u64s(&format!("{tmp}/ids.u64"), &ids)?;
    // check sorted / self loops
    let csr = Csr::open(&tmp)?;
    let (sorted, loops) = (0..n).into_par_iter().map(|v| {
        let l = csr.list(v);
        let s = l.windows(2).all(|w| w[0] < w[1]);
        let sl = l.binary_search(&(v as u32)).is_ok();
        (s, sl as u64)
    }).reduce(|| (true, 0), |a, b| (a.0 && b.0, a.1 + b.1));
    let meta = json!({"format":"glab-csr-v0","n":n,"m_undirected":null,"entries":m,"arcs":m,
        "directed":true,"symmetric":false,"sorted":sorted,"self_loops":loops>0,"self_loop_count":loops,
        "source":"LAW twitter-2010 BVGraph (twitter-2010.graph), node ids = BV ids (ids.u64 is identity)",
        "note":"out-adjacency; strictly increasing per node (deduplicated) iff sorted=true"});
    std::fs::write(format!("{tmp}/meta.json"), serde_json::to_string_pretty(&meta)?)?;
    std::fs::rename(&tmp, outdir)?;
    eprintln!("bv2csr done {:?} sorted={sorted} loops={loops}", t0.elapsed());
    Ok(())
}

// ---------------------------------------------------------------- symmetrize
fn symmetrize(indir: &str, outdir: &str) -> Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    let t0 = Instant::now();
    let g = Csr::open(indir)?;
    let n = g.n;
    // in-degrees
    let indeg: Vec<AtomicU64> = (0..n).map(|_| AtomicU64::new(0)).collect();
    let chunks = make_chunks(&g.offsets, 4096);
    chunks.par_iter().for_each(|&(lo, hi)| {
        for v in lo..hi {
            for &u in g.list(v) {
                indeg[u as usize].fetch_add(1, Relaxed);
            }
        }
    });
    let mut ioff = vec![0u64; n + 1];
    for v in 0..n {
        ioff[v + 1] = ioff[v] + indeg[v].load(Relaxed);
    }
    for v in 0..n {
        indeg[v].store(ioff[v], Relaxed);
    }
    let mi = ioff[n] as usize;
    let mut itg = vec![0u32; mi];
    let ip = SendPtr(itg.as_mut_ptr());
    chunks.par_iter().for_each(|&(lo, hi)| {
        let ip = &ip;
        for v in lo..hi {
            for &u in g.list(v) {
                let p = indeg[u as usize].fetch_add(1, Relaxed);
                unsafe { *ip.0.add(p as usize) = v as u32 };
            }
        }
    });
    drop(indeg);
    eprintln!("transpose scatter {:?}", t0.elapsed());
    let ichunks = make_chunks(&ioff, 4096);
    {
        let ip = SendPtr(itg.as_mut_ptr());
        ichunks.par_iter().for_each(|&(lo, hi)| {
            let ip = &ip;
            for v in lo..hi {
                let s = unsafe { std::slice::from_raw_parts_mut(ip.0.add(ioff[v] as usize), (ioff[v + 1] - ioff[v]) as usize) };
                s.sort_unstable();
            }
        });
    }
    eprintln!("transpose sorted {:?}", t0.elapsed());
    let merge = |v: usize, mut out: Option<&mut [u32]>| -> usize {
        let a = g.list(v);
        let b = &itg[ioff[v] as usize..ioff[v + 1] as usize];
        let (mut i, mut j, mut k) = (0, 0, 0);
        let mut last = u32::MAX;
        while i < a.len() || j < b.len() {
            let x = if j >= b.len() || (i < a.len() && a[i] <= b[j]) { i += 1; a[i - 1] } else { j += 1; b[j - 1] };
            if x == last || x as usize == v { continue; }
            last = x;
            if let Some(o) = out.as_deref_mut() { o[k] = x; }
            k += 1;
        }
        k
    };
    let deg: Vec<u64> = (0..n).into_par_iter().map(|v| merge(v, None) as u64).collect();
    let mut soff = vec![0u64; n + 1];
    for v in 0..n {
        soff[v + 1] = soff[v] + deg[v];
    }
    drop(deg);
    let ms = soff[n] as usize;
    eprintln!("sym entries {ms} {:?}", t0.elapsed());
    let tmp = format!("{outdir}.tmp");
    std::fs::create_dir_all(&tmp)?;
    let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(format!("{tmp}/targets.u32"))?;
    f.set_len((ms * 4) as u64)?;
    let mut mm = unsafe { memmap2::MmapMut::map_mut(&f)? };
    let op = SendPtr(mm.as_mut_ptr() as *mut u32);
    let schunks = make_chunks(&soff, 4096);
    schunks.par_iter().for_each(|&(lo, hi)| {
        let op = &op;
        for v in lo..hi {
            let o = unsafe { std::slice::from_raw_parts_mut(op.0.add(soff[v] as usize), (soff[v + 1] - soff[v]) as usize) };
            let k = merge(v, Some(o));
            assert_eq!(k as u64, soff[v + 1] - soff[v]);
        }
    });
    mm.flush()?;
    drop(mm);
    write_u64s(&format!("{tmp}/offsets.u64"), &soff)?;
    let ids: Vec<u64> = (0..n as u64).collect();
    write_u64s(&format!("{tmp}/ids.u64"), &ids)?;
    let meta = json!({"format":"glab-csr-v0","n":n,"m_undirected":ms/2,"entries":ms,
        "directed":false,"symmetric":true,"sorted":true,"self_loops":false,
        "source":format!("symmetrized from {indir} (union of out- and in-arcs, dedup, self-loops removed); ids = twitter-2010 BV ids")});
    std::fs::write(format!("{tmp}/meta.json"), serde_json::to_string_pretty(&meta)?)?;
    std::fs::rename(&tmp, outdir)?;
    eprintln!("sym done {:?}", t0.elapsed());
    Ok(())
}

// ---------------------------------------------------------------- csr2bv
fn csr2bv(csrdir: &str, basename: &str, window: usize, maxref: usize) -> Result<()> {
    let t0 = Instant::now();
    let g = Csr::open(csrdir)?;
    let dcf: Box<[usize]> = g.offsets.iter().map(|&x| x as usize).collect();
    // zero-copy: view the little-endian u32 targets as a 32-bit BitFieldVec over usize words
    assert!(g.m % 2 == 0 && (g.m * 4) % 4096 != 0, "need even m (and file not page-aligned) for zero-copy view");
    let words: &'static [usize] = unsafe { std::slice::from_raw_parts(g.targets.as_ptr() as *const usize, g.m / 2) };
    let succ = unsafe { sux::bits::BitFieldVec::from_raw_parts(words, 32, g.m) };
    let csr = unsafe { CsrGraph::from_parts(dcf, succ) };
    let mut flags = CompFlags::default();
    flags.compression_window = window;
    flags.max_ref_count = maxref;
    let bits = BvComp::with_basename(basename).comp_flags(flags).par_comp::<BE, _>(&csr)?;
    let (cs, cn) = g.checksum();
    std::fs::write(format!("{basename}.truth"), format!("{cs} {cn}\n"))?;
    eprintln!("csr2bv {basename} bits={bits} bits/arc={:.3} {:?}", bits as f64 / g.m as f64, t0.elapsed());
    Ok(())
}

// ---------------------------------------------------------------- encode
fn encode_cmd(csrdir: &str, codec: &str, prefix: &str) -> Result<()> {
    let t0 = Instant::now();
    let g = Csr::open(csrdir)?;
    let c = Codec::parse(codec)?;
    let enc = encode(&g, c);
    let secs = t0.elapsed().as_secs_f64();
    enc.write(prefix)?;
    let (cs, cn) = g.checksum();
    std::fs::write(format!("{prefix}.truth"), format!("{cs} {cn}\n"))?;
    eprintln!("encode {codec}: {} bytes data, {:.3} bits/edge (data only) in {:.2}s", enc.data.len(), enc.data.len() as f64 * 8.0 / g.m as f64, secs);
    Ok(())
}

// ---------------------------------------------------------------- bench
fn bench_cmd(a: &[String]) -> Result<()> {
    // bench <csrdir> <codec> <prefix-or-bvbasename> <threads> <out.json> [cold] [rand_samples]
    let csrdir = &a[0];
    let codec = a[1].as_str();
    let prefix = &a[2];
    let threads: usize = a[3].parse()?;
    let out = &a[4];
    let cold = a.get(5).map(|s| s == "cold").unwrap_or(false);
    let only_cold = a.get(5).map(|s| s == "coldonly").unwrap_or(false);
    let rand_samples: usize = a.get(6).map(|s| s.parse().unwrap()).unwrap_or(200_000);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build()?;
    let g = Csr::open(csrdir)?;
    let mut res = json!({"csr": csrdir, "codec": codec, "prefix": prefix, "threads": threads, "n": g.n, "m": g.m});
    let r = if codec.starts_with("bv") {
        let gb = std::fs::metadata(format!("{prefix}.graph"))?.len();
        let efb = std::fs::metadata(format!("{prefix}.ef"))?.len();
        // byte ranges of chunks from offset_deg_iter
        let seq = BvGraphSeq::with_basename(prefix).endianness::<BE>().load()?;
        let mut boff = Vec::with_capacity(g.n + 1);
        let mut it = seq.offset_deg_iter();
        for _ in 0..g.n {
            boff.push(it.get_pos() / 8);
            it.next_degree()?;
        }
        boff.push(it.get_pos() / 8 + 1);
        drop(it);
        drop(seq);
        // evict before webgraph maps the file again (DONTNEED cannot drop pages that are currently mapped)
        if cold || only_cold {
            evict(&format!("{prefix}.graph"))?;
        }
        res["data_bytes"] = json!(gb);
        res["index_bytes"] = json!(efb);
        res["index_note"] = json!("webgraph .ef (Elias-Fano over bit offsets)");
        if codec == "bvs" {
            let bv = BvGraph::with_basename(prefix).endianness::<BE>().dispatch::<Static>().load()?;
            run_all(&pool, &g, &BvLists { g: &bv }, &mut res, &format!("{prefix}.graph"), &boff, cold, only_cold, rand_samples)?
        } else {
            let bv = BvGraph::with_basename(prefix).endianness::<BE>().load()?;
            run_all(&pool, &g, &BvLists { g: &bv }, &mut res, &format!("{prefix}.graph"), &boff, cold, only_cold, rand_samples)?
        }
    } else {
        let c = Codec::parse(codec)?;
        let enc = Encoded::load(prefix, c, g.n)?;
        res["data_bytes"] = json!(enc.data.len());
        res["index_bytes"] = json!(enc.index_bytes());
        res["index_note"] = json!(enc.index_note());
        let boff: Vec<u64> = enc.byte_off_per_node();
        run_all(&pool, &g, &enc, &mut res, &enc.data_path(prefix), &boff, cold, only_cold, rand_samples)?
    };
    let _ = r;
    let m = g.m as f64;
    let total = res["data_bytes"].as_u64().unwrap() + res["index_bytes"].as_u64().unwrap();
    res["bits_per_edge"] = json!(total as f64 * 8.0 / m);
    res["bits_per_edge_data_only"] = json!(res["data_bytes"].as_u64().unwrap() as f64 * 8.0 / m);
    println!("{}", serde_json::to_string(&res)?);
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(out)?;
    use std::io::Write;
    writeln!(f, "{}", serde_json::to_string(&res)?)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_all<L: Lists + Sync>(pool: &rayon::ThreadPool, g: &Csr, lists: &L, res: &mut serde_json::Value, datapath: &str, boff: &[u64], cold: bool, only_cold: bool, rand_samples: usize) -> Result<()> {
    let n = g.n;
    let m = g.m as f64;
    let chunks = lists.work_chunks(make_chunks(&g.offsets, 1 << 14));
    // per-encoding truth checksum written at encode time (avoids touching the raw CSR targets)
    let tdir = Path::new(&datapath.replace(".bin", "").replace(".graph", "")).with_extension("truth");
    let tdir = if datapath.ends_with(".bin") || datapath.ends_with(".graph") { Path::new(&format!("{}.truth", datapath.trim_end_matches(".bin").trim_end_matches(".graph"))).to_path_buf() } else { tdir };
    let truth = if tdir.exists() {
        let t = std::fs::read_to_string(&tdir)?;
        let v: Vec<u64> = t.split_whitespace().map(|x| x.parse().unwrap()).collect();
        (v[0], v[1])
    } else {
        g.checksum()
    };
    if cold || only_cold {
        // cold: evict data file pages, then parallel scan with WILLNEED prefetch per chunk
        let ev = evict(datapath)?;
        let t = Instant::now();
        let cs = pool.install(|| scan_par(lists, &chunks, Some((datapath, boff))));
        let s = t.elapsed().as_secs_f64();
        if cs != truth { bail!("cold checksum mismatch {cs:?} vs {truth:?}"); }
        res["cold_scan_s"] = json!(s);
        res["cold_scan_edges_per_s"] = json!(m / s);
        res["cold_resident_before"] = json!(ev);
        let t = Instant::now();
        let cs = pool.install(|| scan_par(lists, &chunks, None));
        let s = t.elapsed().as_secs_f64();
        assert_eq!(cs, truth);
        res["after_cold_rescan_edges_per_s"] = json!(m / s);
        if only_cold { return Ok(()); }
    }
    // warm-up
    let cs = pool.install(|| scan_par(lists, &chunks, None));
    if cs != truth { bail!("checksum mismatch {cs:?} vs {truth:?}"); }
    // parallel scan (best of 3)
    let mut best = f64::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        let cs = pool.install(|| scan_par(lists, &chunks, None));
        best = best.min(t.elapsed().as_secs_f64());
        assert_eq!(cs, truth);
    }
    res["scan_par_s"] = json!(best);
    res["scan_par_edges_per_s"] = json!(m / best);
    // 1-thread scan
    let t = Instant::now();
    let cs = scan_range(lists, 0, n);
    let s1 = t.elapsed().as_secs_f64();
    assert_eq!(cs, truth);
    res["scan_1t_s"] = json!(s1);
    res["scan_1t_edges_per_s"] = json!(m / s1);
    // pagerank pull iteration
    let deg: Vec<u32> = (0..n).map(|v| (g.offsets[v + 1] - g.offsets[v]) as u32).collect();
    let mut pr = vec![1.0f32 / n as f32; n];
    let mut x = vec![0f32; n];
    let mut best = f64::MAX;
    let mut sum = 0.0;
    for _ in 0..3 {
        let t = Instant::now();
        pool.install(|| pr_iter(lists, &chunks, &deg, &mut pr, &mut x));
        best = best.min(t.elapsed().as_secs_f64());
        sum = pr.iter().map(|&v| v as f64).sum::<f64>();
    }
    res["pr_iter_s"] = json!(best);
    res["pr_edges_per_s"] = json!(m / best);
    res["pr_sum_check"] = json!(sum);
    // random access, 1 thread
    let mut rng = 0x9E3779B97F4A7C15u64;
    let nodes: Vec<usize> = (0..rand_samples).map(|_| { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; (rng % n as u64) as usize }).collect();
    let mut buf = Vec::new();
    let mut tot = 0u64;
    let mut chk = 0u64;
    let t = Instant::now();
    for &v in &nodes {
        let l = lists.get(v, &mut buf);
        tot += l.len() as u64;
        chk = chk.wrapping_add(l.last().copied().unwrap_or(0) as u64);
    }
    let s = t.elapsed().as_secs_f64();
    std::hint::black_box(chk);
    res["rand_samples"] = json!(rand_samples);
    res["rand_ns_per_list"] = json!(s * 1e9 / rand_samples as f64);
    res["rand_ns_per_edge"] = json!(s * 1e9 / tot.max(1) as f64);
    // random access, all threads (throughput)
    let t = Instant::now();
    let tot2: u64 = pool.install(|| nodes.par_chunks(1024).map(|ch| {
        let mut buf = Vec::new();
        ch.iter().map(|&v| lists.get(v, &mut buf).len() as u64).sum::<u64>()
    }).sum());
    let s = t.elapsed().as_secs_f64();
    assert_eq!(tot, tot2);
    res["rand_par_ns_per_list"] = json!(s * 1e9 / rand_samples as f64);
    Ok(())
}

fn evict(path: &str) -> Result<u64> {
    use std::os::fd::AsRawFd;
    let f = std::fs::File::open(path).context(path.to_string())?;
    unsafe {
        libc::fdatasync(f.as_raw_fd());
        libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
    Ok(resident_pages(path)?)
}

fn resident_pages(path: &str) -> Result<u64> {
    let f = std::fs::File::open(path)?;
    let len = f.metadata()?.len() as usize;
    if len == 0 { return Ok(0); }
    let mm = unsafe { memmap2::Mmap::map(&f)? };
    let pages = len.div_ceil(4096);
    let mut v = vec![0u8; pages];
    unsafe { libc::mincore(mm.as_ptr() as *mut _, len, v.as_mut_ptr()) };
    Ok(v.iter().filter(|&&b| b & 1 == 1).count() as u64)
}

fn scan_par<L: Lists + Sync>(lists: &L, chunks: &[(usize, usize)], prefetch: Option<(&str, &[u64])>) -> (u64, u64) {
    let mm = prefetch.map(|(p, _)| unsafe { memmap2::Mmap::map(&std::fs::File::open(p).unwrap()).unwrap() });
    chunks.par_iter().map(|&(lo, hi)| {
        if let (Some(mm), Some((_, boff))) = (&mm, prefetch) {
            let b0 = (boff[lo] as usize) & !4095;
            let b1 = (boff[hi] as usize).min(mm.len());
            if b1 > b0 {
                unsafe { libc::madvise(mm.as_ptr().add(b0) as *mut _, b1 - b0, libc::MADV_WILLNEED) };
            }
        }
        scan_range(lists, lo, hi)
    }).reduce(|| (0, 0), |a, b| (a.0.wrapping_add(b.0), a.1 + b.1))
}

fn scan_range<L: Lists>(lists: &L, lo: usize, hi: usize) -> (u64, u64) {
    let mut s = 0u64;
    let mut c = 0u64;
    let mut buf = Vec::new();
    lists.for_range(lo, hi, &mut buf, |_, l| {
        c += l.len() as u64;
        for &x in l {
            s = s.wrapping_add(x as u64);
        }
    });
    (s, c)
}

fn pr_iter<L: Lists + Sync>(lists: &L, chunks: &[(usize, usize)], deg: &[u32], pr: &mut [f32], x: &mut [f32]) {
    let n = pr.len();
    let contrib: Vec<f32> = pr.par_iter().zip(deg.par_iter()).map(|(&p, &d)| if d > 0 { p / d as f32 } else { 0.0 }).collect();
    let xp = SendPtr(x.as_mut_ptr());
    chunks.par_iter().for_each(|&(lo, hi)| {
        let xp = &xp;
        let mut buf = Vec::new();
        lists.for_range(lo, hi, &mut buf, |v, l| {
            let mut s = 0f32;
            for &u in l {
                s += unsafe { *contrib.get_unchecked(u as usize) };
            }
            unsafe { *xp.0.add(v) = s };
        });
    });
    let d = 0.85f32;
    pr.par_iter_mut().zip(x.par_iter()).for_each(|(p, &xv)| *p = (1.0 - d) / n as f32 + d * xv);
}

// BV adapter
struct BvLists<'a, G> {
    g: &'a G,
}
impl<G: RandomAccessGraph + Sync> Lists for BvLists<'_, G> {
    fn for_range<F: FnMut(usize, &[u32])>(&self, lo: usize, hi: usize, buf: &mut Vec<u32>, mut f: F) {
        let mut l = self.g.iter_from(lo);
        for _ in lo..hi {
            let (v, succ) = l.next().unwrap();
            buf.clear();
            buf.extend(succ.into_iter().map(|x| x as u32));
            f(v, buf);
        }
    }
    fn get<'b>(&self, v: usize, buf: &'b mut Vec<u32>) -> &'b [u32] {
        buf.clear();
        buf.extend(self.g.successors(v).into_iter().map(|x| x as u32));
        buf
    }
}

// ---------------------------------------------------------------- permutations
fn read_perm(path: &str) -> Result<Vec<u32>> {
    let b = std::fs::read(path)?;
    Ok(b.chunks_exact(8).map(|c| u64::from_be_bytes(c.try_into().unwrap()) as u32).collect())
}
fn write_perm(path: &str, p: &[u32]) -> Result<()> {
    let mut b = Vec::with_capacity(p.len() * 8);
    for &x in p { b.extend_from_slice(&(x as u64).to_be_bytes()); }
    std::fs::write(path, b)?;
    Ok(())
}

/// perm[old] = new
fn perm_cmd(csrdir: &str, permfile: &str, outdir: &str) -> Result<()> {
    let t0 = Instant::now();
    let g = Csr::open(csrdir)?;
    let p = read_perm(permfile)?;
    let n = g.n;
    assert_eq!(p.len(), n);
    let mut inv = vec![0u32; n];
    for (o, &nw) in p.iter().enumerate() { inv[nw as usize] = o as u32; }
    let mut off = vec![0u64; n + 1];
    for v in 0..n { let o = inv[v] as usize; off[v + 1] = off[v] + g.offsets[o + 1] - g.offsets[o]; }
    let mut tg = vec![0u32; g.m];
    let tp = SendPtr(tg.as_mut_ptr());
    let ch = make_chunks(&off, 4096);
    ch.par_iter().for_each(|&(lo, hi)| {
        let tp = &tp;
        for v in lo..hi {
            let s = unsafe { std::slice::from_raw_parts_mut(tp.0.add(off[v] as usize), (off[v + 1] - off[v]) as usize) };
            for (d, &u) in s.iter_mut().zip(g.list(inv[v] as usize)) { *d = p[u as usize]; }
            s.sort_unstable();
        }
    });
    std::fs::create_dir_all(outdir)?;
    write_u64s(&format!("{outdir}/offsets.u64"), &off)?;
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(tg.as_ptr() as *const u8, tg.len() * 4) };
    std::fs::write(format!("{outdir}/targets.u32"), bytes)?;
    let meta = json!({"format":"glab-csr-v0","n":n,"entries":g.m,"permuted_from":csrdir,"perm":permfile});
    std::fs::write(format!("{outdir}/meta.json"), serde_json::to_string_pretty(&meta)?)?;
    eprintln!("perm done {:?}", t0.elapsed());
    Ok(())
}

/// order kinds: deg (degree descending), lp (label propagation community id, then BFS-ish within by original id)
fn order_cmd(csrdir: &str, kind: &str, out: &str) -> Result<()> {
    let t0 = Instant::now();
    let g = Csr::open(csrdir)?;
    let n = g.n;
    let deg = |v: usize| g.offsets[v + 1] - g.offsets[v];
    let mut order: Vec<u32> = (0..n as u32).collect();
    match kind {
        "rand" => {
            let mut r = 0x2545F4914F6CDD1Du64;
            for i in (1..n).rev() {
                r ^= r << 13; r ^= r >> 7; r ^= r << 17;
                order.swap(i, (r % (i as u64 + 1)) as usize);
            }
        }
        "deg" => order.par_sort_by_key(|&v| (std::cmp::Reverse(deg(v as usize)), v)),
        "lp" | "lp2" => {
            // synchronous-free (asynchronous, chunk-parallel) label propagation, few rounds
            use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
            let lab: Vec<AtomicU32> = (0..n as u32).map(AtomicU32::new).collect();
            let rounds = 10;
            for r in 0..rounds {
                let changed: u64 = (0..n).into_par_iter().with_min_len(4096).map_init(|| std::collections::HashMap::<u32, u32>::new(), |h, v| {
                    let l = g.list(v);
                    if l.is_empty() { return 0; }
                    h.clear();
                    for &u in l { *h.entry(lab[u as usize].load(Relaxed)).or_insert(0) += 1; }
                    let cur = lab[v].load(Relaxed);
                    let mut best = (h.get(&cur).copied().unwrap_or(0), cur);
                    for (&k, &c) in h.iter() { if c > best.0 || (c == best.0 && k < best.1) { best = (c, k); } }
                    if best.1 != cur { lab[v].store(best.1, Relaxed); 1 } else { 0 }
                }).sum();
                eprintln!("lp round {r} changed {changed} {:?}", t0.elapsed());
            }
            let lab: Vec<u32> = lab.into_iter().map(|a| a.into_inner()).collect();
            if kind == "lp" {
                order.par_sort_by_key(|&v| (lab[v as usize], v));
            } else {
                // lp2: second level — communities ordered by their smallest member; within: BFS restricted to community
                order.par_sort_by_key(|&v| (lab[v as usize], v));
            }
            let mut c = lab.clone(); c.par_sort_unstable(); c.dedup();
            eprintln!("lp communities {}", c.len());
        }
        "bfs" => {
            let mut seen = vec![false; n];
            let mut q = std::collections::VecDeque::new();
            let mut k = 0;
            let mut starts: Vec<u32> = (0..n as u32).collect();
            starts.par_sort_by_key(|&v| (std::cmp::Reverse(deg(v as usize)), v));
            for &s in &starts {
                if seen[s as usize] { continue; }
                seen[s as usize] = true; q.push_back(s);
                while let Some(v) = q.pop_front() {
                    order[k] = v; k += 1;
                    for &u in g.list(v as usize) { if !seen[u as usize] { seen[u as usize] = true; q.push_back(u); } }
                }
            }
        }
        _ => bail!("kind"),
    }
    // order[newpos] = old  -> perm[old] = new
    let mut p = vec![0u32; n];
    for (i, &o) in order.iter().enumerate() { p[o as usize] = i as u32; }
    write_perm(out, &p)?;
    eprintln!("order {kind} done {:?}", t0.elapsed());
    Ok(())
}

// ---------------------------------------------------------------- id width
/// Encode the same lists with ids shifted by 2^32 (u64) to show varint / bitpacking cost depends on gaps not id width.
fn idwidth_cmd(csrdir: &str) -> Result<()> {
    let g = Csr::open(csrdir)?;
    let shift: u64 = 1 << 33;
    let (mut b32, mut b64) = (0usize, 0usize);
    let mut buf = Vec::new();
    for v in 0..g.n {
        let l = g.list(v);
        buf.clear();
        varint_list_u64(v as u64, l.iter().map(|&x| x as u64), &mut buf);
        b32 += buf.len();
        buf.clear();
        varint_list_u64(v as u64 + shift, l.iter().map(|&x| x as u64 + shift), &mut buf);
        b64 += buf.len();
    }
    // bit-packing: width needed = max gap bits, independent of base
    let mut w32 = 0u64; let mut w64 = 0u64;
    for v in 0..g.n {
        let l = g.list(v);
        for ch in l.chunks(128) {
            if ch.len() < 2 { continue; }
            let mg = ch.windows(2).map(|w| (w[1] - w[0]) as u64).max().unwrap();
            let mg64 = ch.windows(2).map(|w| (w[1] as u64 + shift) - (w[0] as u64 + shift)).max().unwrap();
            w32 += (64 - mg.leading_zeros()) as u64 * ch.len() as u64;
            w64 += (64 - mg64.leading_zeros()) as u64 * ch.len() as u64;
        }
    }
    let r = json!({"csr":csrdir,"varint_bits_per_edge_u32ids":b32 as f64*8.0/g.m as f64,"varint_bits_per_edge_u64ids_shifted_2^33":b64 as f64*8.0/g.m as f64,
        "bp128_payload_bits_u32":w32 as f64/g.m as f64,"bp128_payload_bits_u64":w64 as f64/g.m as f64,
        "raw_u32_bits":32,"raw_u64_bits":64});
    println!("{}", r);
    Ok(())
}

fn checksym(symdir: &str, dirdir: &str) -> Result<()> {
    let s = Csr::open(symdir)?;
    let d = Csr::open(dirdir)?;
    let bad: u64 = (0..s.n).into_par_iter().map(|u| {
        let l = s.list(u);
        let mut b = 0u64;
        if !l.windows(2).all(|w| w[0] < w[1]) || l.binary_search(&(u as u32)).is_ok() { b += 1; }
        for &v in l { if s.list(v as usize).binary_search(&(u as u32)).is_err() { b += 1; } }
        for &v in d.list(u) { if v as usize != u && l.binary_search(&v).is_err() { b += 1; } }
        b
    }).sum();
    println!("checksym bad={bad}");
    Ok(())
}

/// O_DIRECT parallel read bandwidth of a file (4 MiB requests, `t` threads).
fn readbw(path: &str, t: usize) -> Result<()> {
    use std::os::unix::fs::{FileExt, OpenOptionsExt};
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECT).open(path)?;
    let len = f.metadata()?.len();
    const B: u64 = 4 << 20;
    let nb = len.div_ceil(B);
    let next = std::sync::atomic::AtomicU64::new(0);
    let t0 = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..t {
            s.spawn(|| {
                let layout = std::alloc::Layout::from_size_align(B as usize, 4096).unwrap();
                let p = unsafe { std::alloc::alloc(layout) };
                let buf = unsafe { std::slice::from_raw_parts_mut(p, B as usize) };
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= nb { break; }
                    let _ = f.read_at(buf, i * B);
                }
            });
        }
    });
    let s = t0.elapsed().as_secs_f64();
    println!("{}", json!({"file":path,"bytes":len,"threads":t,"secs":s,"GBps":len as f64/s/1e9}));
    Ok(())
}
