//! ldbc-lab: the lab kernels of lab/b1-core (PageRank f64 and f32, WCC, BFS) and the
//! oracle's specification algorithms, checked against the LDBC Graphalytics reference
//! outputs with the specification's match rules (issue #177).
//!
//!   ldbc-lab build --graph DIR --out CSR
//!       Reads an undirected LDBC graph and writes ids.u64, offsets.u64, targets.u32
//!       (the glab-csr-v0 layout, dense ids in increasing LDBC id order).
//!   ldbc-lab check --graph DIR [--csr CSR --load heap|mmap|stream] [--lab pr,wcc,bfs]
//!       [--spec bfs,pr,wcc,cdlp,lcc,sssp] [--chunk ENTRIES] [--json FILE]
//!       Without --csr the graph is read from the LDBC files into RAM.
//!       The CSR files are evicted from the page cache first. heap: CSR files read into
//!       RAM; mmap: CSR files mapped; stream: PageRank and WCC
//!       from extmem.rs (targets read in chunks, O(n) state in RAM), BFS on the map.
//!
//! These are not LDBC Benchmark Results. Times are exploratory.

use everygraph_oracle::ldbc::read::merge_directions;
use everygraph_oracle::ldbc::run::{self, Row};
use everygraph_oracle::ldbc::{spec, Algorithm, Dataset, Ids, LdbcGraph, Lists, Values, UNREACHABLE};
use glab_core::extmem::{self, EdgeStream, IoMode};
use glab_core::{edge_chunks, kernels, Csr, LoadMode};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::Instant;

fn die(msg: String) -> ! {
    eprintln!("ldbc-lab: {msg}");
    std::process::exit(2);
}

struct Args(Vec<String>);
impl Args {
    fn get(&self, k: &str) -> Option<String> {
        self.0.iter().position(|a| a == k).and_then(|i| self.0.get(i + 1)).cloned()
    }
    fn algos(&self, k: &str, default: &str) -> Vec<Algorithm> {
        let v = self.get(k).unwrap_or_else(|| default.to_owned());
        v.split(',').filter(|s| !s.is_empty()).map(|s| Algorithm::parse(s).unwrap_or_else(|| die(format!("unknown algorithm {s}")))).collect()
    }
}

fn write_raw<T: Copy>(path: &Path, v: &[T]) {
    // Little-endian plain arrays, as glab-core's loader reads them.
    let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) };
    let mut f = std::io::BufWriter::with_capacity(1 << 24, std::fs::File::create(path).unwrap_or_else(|e| die(format!("{}: {e}", path.display()))));
    f.write_all(bytes).and_then(|_| f.flush()).unwrap_or_else(|e| die(format!("{}: {e}", path.display())));
}

fn read_u64s(path: &Path) -> Vec<u64> {
    let b = std::fs::read(path).unwrap_or_else(|e| die(format!("{}: {e}", path.display())));
    b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect()
}

/// Everything the kernels read. `out` is the lab CSR (symmetric when undirected).
struct Loaded {
    ids: Ids,
    directed: bool,
    out: Csr,
    /// Directed graphs only: in-lists (spec PageRank and CDLP) and merged lists with arcs.
    inn: Option<everygraph_oracle::ldbc::Csr>,
    sym: Option<(Csr, Vec<u8>)>,
    weights: Option<Vec<f64>>,
    targets_file: Option<PathBuf>,
}

impl Loaded {
    fn from_ldbc(g: LdbcGraph) -> Loaded {
        let LdbcGraph { ids, directed, out, inn, .. } = g;
        let sym = inn.as_ref().map(|i| {
            let (c, d) = merge_directions(out.lists(), i.lists());
            (Csr::from_vecs(c.offsets, c.targets), d)
        });
        Loaded { ids, directed, out: Csr::from_vecs(out.offsets, out.targets), inn, sym, weights: out.weights, targets_file: None }
    }

    fn from_csr(dir: &Path, mode: LoadMode) -> Loaded {
        let ids = Ids::new(read_u64s(&dir.join("ids.u64"))).unwrap_or_else(|e| die(format!("ids: {e}")));
        let out = Csr::load(dir, mode).unwrap_or_else(|e| die(format!("{}: {e}", dir.display())));
        Loaded { ids, directed: false, out, inn: None, sym: None, weights: None, targets_file: Some(dir.join("targets.u32")) }
    }

    fn out_lists(&self) -> Lists<'_> {
        Lists { offsets: &self.out.offsets, targets: &self.out.targets, weights: self.weights.as_deref() }
    }

    fn sym_lists(&self) -> (Lists<'_>, Option<&[u8]>) {
        match &self.sym {
            Some((c, d)) => (Lists { offsets: &c.offsets, targets: &c.targets, weights: None }, Some(d)),
            None => (self.out_lists(), None),
        }
    }

    fn sym_csr(&self) -> &Csr {
        self.sym.as_ref().map_or(&self.out, |s| &s.0)
    }

    fn labels(&self, labels: impl Iterator<Item = u32>) -> Values {
        Values::Ints(labels.map(|l| self.ids.ids[l as usize]).collect())
    }
}

fn source(ds: &Dataset, g: &Loaded, a: Algorithm) -> Result<u32, String> {
    let id = ds.source(a).map_err(|e| e.to_string())?;
    g.ids.index(id).ok_or_else(|| format!("source {id} is not a vertex"))
}

fn run_spec(ds: &Dataset, g: &Loaded, a: Algorithm) -> Result<Values, String> {
    let e = |e: std::io::Error| e.to_string();
    let inn = g.inn.as_ref().map(|c| c.lists());
    Ok(match a {
        Algorithm::Bfs => Values::Ints(spec::bfs(g.out_lists(), source(ds, g, a)?)),
        Algorithm::Pr => Values::Reals(spec::pagerank(g.out_lists(), inn.unwrap_or(g.out_lists()), ds.damping().map_err(e)?, ds.iterations(a).map_err(e)?)),
        Algorithm::Wcc => g.labels(spec::wcc(g.sym_lists().0).into_iter()),
        Algorithm::Cdlp => {
            let identity: Vec<u32> = (0..g.ids.len() as u32).collect();
            g.labels(spec::cdlp(g.out_lists(), inn, &identity, ds.iterations(a).map_err(e)?).into_iter())
        }
        Algorithm::Lcc => {
            let (l, d) = g.sym_lists();
            Values::Reals(spec::lcc(l, d))
        }
        Algorithm::Sssp => Values::Reals(spec::sssp(g.out_lists(), source(ds, g, a)?).ok_or("skipped: no weights loaded")?),
    })
}

/// FNV-1a over the result's bit patterns: equal hashes across loads mean the same bits.
fn bits_hash(v: &Values) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |x: u64| {
        for b in x.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    match v {
        Values::Ints(x) => x.iter().for_each(|&y| eat(y)),
        Values::Reals(x) => x.iter().for_each(|y| eat(y.to_bits())),
    }
    format!("{h:016x}")
}

fn atomics(v: &[AtomicU32]) -> impl Iterator<Item = u32> + '_ {
    v.iter().map(|x| x.load(Relaxed))
}

/// The lab kernels for `a`: (name, result).
fn run_lab(ds: &Dataset, g: &Loaded, a: Algorithm, load: &str, chunk: u64) -> Vec<(String, Result<Values, String>, f64)> {
    let mut out = Vec::new();
    let mut go = |name: &str, f: &mut dyn FnMut() -> Result<Values, String>| {
        let t = Instant::now();
        let r = f();
        out.push((format!("lab-{name}/{load}"), r, t.elapsed().as_secs_f64()));
    };
    let stream = load == "stream";
    let es = |c: &Csr, chunks: &[(usize, usize)]| {
        let path = g.targets_file.as_ref().expect("stream needs --csr");
        EdgeStream::open(path, IoMode::Buffered, &c.offsets, chunks).unwrap_or_else(|e| die(format!("{e}")))
    };
    match a {
        Algorithm::Bfs => go("bfs-do", &mut || {
            let chunks = edge_chunks(&g.out.offsets, chunk);
            let r = kernels::bfs_do(&g.out, source(ds, g, a)? as usize, &chunks);
            Ok(Values::Ints(atomics(&r.depth).map(|d| if d == u32::MAX { UNREACHABLE } else { u64::from(d) }).collect()))
        }),
        Algorithm::Pr => {
            let unsupported = || Err::<Values, String>("skipped: the lab PageRank pulls over the lists it divides by, so it needs symmetric lists (undirected graphs only)".into());
            let d = ds.damping().map_err(|e| e.to_string());
            let it = ds.iterations(a).map_err(|e| e.to_string());
            let chunks = edge_chunks(&g.out.offsets, chunk);
            go("pr-f64", &mut || {
                if g.directed {
                    return unsupported();
                }
                let (d, it) = (d.clone()?, it.clone()? as usize);
                let r: Vec<f64> = if stream { extmem::pagerank_semiext::<f64>(&g.out.offsets, &es(&g.out, &chunks), d, it, &chunks).0 } else { kernels::pagerank::<f64, u32>(&g.out.offsets, &g.out.targets, d, it, &chunks) };
                Ok(Values::Reals(r))
            });
            go("pr-f32", &mut || {
                if g.directed {
                    return unsupported();
                }
                let (d, it) = (d.clone()?, it.clone()? as usize);
                let r: Vec<f32> = if stream { extmem::pagerank_semiext::<f32>(&g.out.offsets, &es(&g.out, &chunks), d, it, &chunks).0 } else { kernels::pagerank::<f32, u32>(&g.out.offsets, &g.out.targets, d, it, &chunks) };
                Ok(Values::Reals(r.into_iter().map(f64::from).collect()))
            });
        }
        Algorithm::Wcc => {
            let s = g.sym_csr();
            let chunks = edge_chunks(&s.offsets, chunk);
            if stream {
                go("wcc-semiext", &mut || Ok(g.labels(atomics(&extmem::wcc_semiext(&s.offsets, &es(s, &chunks), &chunks)))));
            } else {
                go("wcc-afforest", &mut || Ok(g.labels(atomics(&kernels::wcc_afforest(s, &chunks).0))));
                go("wcc-stream", &mut || Ok(g.labels(atomics(&kernels::wcc_stream(s, &chunks)))));
            }
        }
        _ => {}
    }
    out
}

fn build(a: &Args) {
    let ds = Dataset::open(Path::new(&a.get("--graph").unwrap_or_else(|| die("--graph".into())))).unwrap_or_else(|e| die(e.to_string()));
    let out = PathBuf::from(a.get("--out").unwrap_or_else(|| die("--out".into())));
    let t = Instant::now();
    let g = ds.load().unwrap_or_else(|e| die(e.to_string()));
    if g.directed {
        die("build writes undirected graphs only".into());
    }
    eprintln!("{}: read {} vertices, {} edges in {:.1} s", ds.name, g.ids.len(), g.edges(), t.elapsed().as_secs_f64());
    std::fs::create_dir_all(&out).unwrap_or_else(|e| die(format!("{e}")));
    write_raw(&out.join("ids.u64"), &g.ids.ids);
    write_raw(&out.join("offsets.u64"), &g.out.offsets);
    write_raw(&out.join("targets.u32"), &g.out.targets);
    let meta = serde_json::json!({"format": "glab-csr-v0", "source": ds.dir, "n": g.ids.len(), "entries": g.out.targets.len(),
        "directed": false, "symmetric": true, "sorted": true, "self_loops": false, "produced_by": "ldbc-lab build (lab/m1-20-ldbc-graphalytics)"});
    std::fs::write(out.join("meta.json"), serde_json::to_string_pretty(&meta).unwrap()).unwrap_or_else(|e| die(format!("{e}")));
    eprintln!("written in {:.1} s total", t.elapsed().as_secs_f64());
}

fn check(a: &Args) {
    let ds = Dataset::open(Path::new(&a.get("--graph").unwrap_or_else(|| die("--graph".into())))).unwrap_or_else(|e| die(e.to_string()));
    let load = a.get("--load").unwrap_or_else(|| "ldbc".into());
    let chunk: u64 = a.get("--chunk").and_then(|s| s.parse().ok()).unwrap_or(1 << 16);
    let lab = a.algos("--lab", "bfs,pr,wcc");
    let specs = a.algos("--spec", "");
    let t = Instant::now();
    let g = match a.get("--csr") {
        None => Loaded::from_ldbc(ds.load().unwrap_or_else(|e| die(e.to_string()))),
        Some(dir) => {
            let mode = match load.as_str() {
                "heap" => LoadMode::Heap,
                "mmap" | "stream" => LoadMode::Mmap,
                other => die(format!("unknown --load {other}")),
            };
            // Page-cache pages stay charged to the cgroup that first read them: without
            // eviction, a budget run would use the cache of an earlier, larger run.
            for f in ["offsets.u64", "targets.u32"] {
                glab_core::evict_file(&Path::new(&dir).join(f)).unwrap_or_else(|e| die(format!("evict {f}: {e}")));
            }
            Loaded::from_csr(Path::new(&dir), mode)
        }
    };
    eprintln!("{}: loaded ({load}) in {:.1} s; threads {}", ds.name, t.elapsed().as_secs_f64(), rayon::current_num_threads());
    let available = ds.algorithms();
    let mut rows: Vec<(Row, Option<String>)> = Vec::new();
    let mut emit = |row: Row, hash: Option<String>| {
        eprintln!("  {}  {}", row.describe(), hash.as_deref().unwrap_or(""));
        rows.push((row, hash));
    };
    for &alg in available.iter() {
        if specs.contains(&alg) {
            let t = Instant::now();
            let got = run_spec(&ds, &g, alg);
            let h = got.as_ref().ok().map(bits_hash);
            emit(run::row(&ds, &g.ids, alg, &format!("spec/{load}"), got, t.elapsed().as_secs_f64()), h);
        }
        if lab.contains(&alg) {
            for (name, got, secs) in run_lab(&ds, &g, alg, &load, chunk) {
                let h = got.as_ref().ok().map(bits_hash);
                emit(run::row(&ds, &g.ids, alg, &name, got, secs), h);
            }
        }
    }
    if let Some(p) = a.get("--json") {
        let v = serde_json::json!({
            "note": "These are not LDBC Benchmark Results. Correctness checks against the LDBC Graphalytics reference outputs; times are exploratory, taken on a shared machine.",
            "graph": ds.name, "load": load, "chunk_entries": chunk, "threads": rayon::current_num_threads(),
            "peak_rss_bytes": glab_core::peak_rss(),
            "rows": rows.iter().map(|(r, h)| { let mut j = r.to_json(); j["bits_fnv1a64"] = h.clone().into(); j }).collect::<Vec<_>>(),
        });
        std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap() + "\n").unwrap_or_else(|e| die(format!("{p}: {e}")));
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cmd = argv.first().cloned().unwrap_or_default();
    let a = Args(argv);
    match cmd.as_str() {
        "build" => build(&a),
        "check" => check(&a),
        _ => die("usage: ldbc-lab build|check ... (see the source header)".into()),
    }
}
