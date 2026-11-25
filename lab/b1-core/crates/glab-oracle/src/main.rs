//! glab-oracle: runs the B1 prototype kernels under the external-kernel protocol of the
//! Everygraph correctness oracle (crates/everygraph-oracle/src/external.rs, issue #6).
//!
//!   glab-oracle [--iterations N] --graph DIR --kernel '<json>' --config '<json>'
//!
//! Reads the graph in the oracle's exchange form and prints one result as JSON. The
//! prototype handles undirected graphs only; self-loops are dropped (its semantics).
//! Regimes map onto the prototype's three code paths:
//!   in_memory  CSR built here from the rows (sort, dedup, symmetrize), Vec-backed;
//!   mmap       the prototype's own ingest (text -> glab-csr-v0 files), loaded with mmap;
//!   stream     the same files; PageRank and WCC from extmem.rs, reading the targets file
//!              in chunks. The prototype has no semi-external BFS, k-core or triangles:
//!              those run on the mmap'd files, as in the mmap regime.
//! The prototype's ingest drops nodes that appear in no row; the adapter re-expands its
//! CSR to the oracle's node ids (rank -> original id) before running anything, so that
//! isolates exist in every regime.

use glab_core::extmem::{self, EdgeStream, IoMode};
use glab_core::kernels;
use glab_core::{edge_chunks, ingest, Csr, LoadMode};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;

fn die(msg: String) -> ! {
    eprintln!("glab-oracle: {msg}");
    std::process::exit(3);
}

struct Input {
    n: usize,
    rows: Vec<(u32, u32)>,
}

fn read_input(dir: &Path) -> Input {
    let desc: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("graph.json")).unwrap_or_else(|e| die(format!("graph.json: {e}"))))
        .unwrap_or_else(|e| die(format!("graph.json: {e}")));
    if desc["directed"].as_bool() != Some(false) {
        die("the prototype handles undirected graphs only".into());
    }
    if desc["semantics"]["self_loops"].as_str() != Some("drop") {
        die("the prototype drops self-loops".into());
    }
    let n = desc["n"].as_u64().unwrap_or_else(|| die("no n".into())) as usize;
    let text = std::fs::read_to_string(dir.join("edges.tsv")).unwrap_or_else(|e| die(format!("edges.tsv: {e}")));
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let mut f = line.split('\t');
        let u: u32 = f.next().and_then(|x| x.parse().ok()).unwrap_or_else(|| die(format!("bad row {line}")));
        let v: u32 = f.next().and_then(|x| x.parse().ok()).unwrap_or_else(|| die(format!("bad row {line}")));
        rows.push((u, v));
    }
    Input { n, rows }
}

/// Symmetric, sorted, duplicate-free CSR without self-loops, over `n` nodes.
fn csr_from_rows(n: usize, rows: &[(u32, u32)]) -> (Vec<u64>, Vec<u32>) {
    let mut e: Vec<(u32, u32)> = Vec::with_capacity(rows.len() * 2);
    for &(u, v) in rows {
        if u != v {
            e.push((u, v));
            e.push((v, u));
        }
    }
    e.sort_unstable();
    e.dedup();
    let mut off = vec![0u64; n + 1];
    for &(u, _) in &e {
        off[u as usize + 1] += 1;
    }
    for i in 0..n {
        off[i + 1] += off[i];
    }
    (off, e.into_iter().map(|x| x.1).collect())
}

fn write_u64s(path: &Path, v: &[u64]) {
    std::fs::write(path, v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()).unwrap_or_else(|e| die(format!("{e}")));
}
fn write_u32s(path: &Path, v: &[u32]) {
    std::fs::write(path, v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()).unwrap_or_else(|e| die(format!("{e}")));
}
fn read_u64s(path: &Path) -> Vec<u64> {
    let b = std::fs::read(path).unwrap_or_else(|e| die(format!("{}: {e}", path.display())));
    b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect()
}
fn read_u32s(path: &Path) -> Vec<u32> {
    let b = std::fs::read(path).unwrap_or_else(|e| die(format!("{}: {e}", path.display())));
    b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

/// Runs the prototype's ingest on the rows and re-expands its CSR to `n` nodes in the
/// oracle's id space; returns the directory holding offsets.u64 and targets.u32.
fn ingest_files(input: &Input, threads: usize, work: &Path) -> PathBuf {
    let text_path = work.join("edges.txt");
    let mut text = String::with_capacity(input.rows.len() * 12);
    for &(u, v) in &input.rows {
        text.push_str(&format!("{u}\t{v}\n"));
    }
    std::fs::write(&text_path, text).unwrap_or_else(|e| die(format!("{e}")));
    let lab_dir = work.join("lab-csr");
    let cfg = ingest::Config {
        input: ingest::Input::File(text_path),
        out_dir: lab_dir.clone(),
        tmp_dir: work.join("tmp"),
        mem_budget: 1 << 30,
        threads,
        fsync: false,
        parse_only: false,
        bucket_mb: 4,
        out_mmap: false,
        direct_in: false,
        direct_tmp: false,
        prealloc: false,
    };
    ingest::run(&cfg).unwrap_or_else(|e| die(format!("prototype ingest failed: {e}")));
    let ids = read_u64s(&lab_dir.join("ids.u64"));
    let off = read_u64s(&lab_dir.join("offsets.u64"));
    let tgt = read_u32s(&lab_dir.join("targets.u32"));
    // Dense lab id i is original id ids[i]; ids are increasing, so lists stay sorted.
    let mut full_off = vec![0u64; input.n + 1];
    for (i, &orig) in ids.iter().enumerate() {
        full_off[orig as usize + 1] = off[i + 1] - off[i];
    }
    for v in 0..input.n {
        full_off[v + 1] += full_off[v];
    }
    let mut full_tgt = vec![0u32; tgt.len()];
    for (i, &orig) in ids.iter().enumerate() {
        let (s, e) = (off[i] as usize, off[i + 1] as usize);
        let at = full_off[orig as usize] as usize;
        for (k, &t) in tgt[s..e].iter().enumerate() {
            full_tgt[at + k] = ids[t as usize] as u32;
        }
    }
    let out = work.join("full");
    std::fs::create_dir_all(&out).unwrap_or_else(|e| die(format!("{e}")));
    write_u64s(&out.join("offsets.u64"), &full_off);
    write_u32s(&out.join("targets.u32"), &full_tgt);
    out
}

fn ints(v: impl Iterator<Item = u32>) -> Value {
    json!({ "ints": v.map(|x| if x == u32::MAX { Value::Null } else { json!(x) }).collect::<Vec<_>>() })
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| argv.iter().position(|a| a == k).and_then(|i| argv.get(i + 1)).cloned();
    let dir = PathBuf::from(get("--graph").unwrap_or_else(|| die("--graph".into())));
    let kernel: Value = serde_json::from_str(&get("--kernel").unwrap_or_else(|| die("--kernel".into()))).unwrap_or_else(|e| die(format!("--kernel: {e}")));
    let config: Value = serde_json::from_str(&get("--config").unwrap_or_else(|| die("--config".into()))).unwrap_or_else(|e| die(format!("--config: {e}")));
    let iters: usize = get("--iterations").and_then(|s| s.parse().ok()).unwrap_or(300);
    let threads = config["threads"].as_u64().unwrap_or(1) as usize;
    let chunk_entries = config["chunk_entries"].as_u64().unwrap_or(1 << 16);
    let regime = config["regime"].as_str().unwrap_or("in_memory").to_owned();
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().unwrap_or_else(|e| die(format!("{e}")));

    let input = read_input(&dir);
    let work = std::env::temp_dir().join(format!("glab-oracle-{}", std::process::id()));
    let name = kernel["kernel"].as_str().unwrap_or("").to_owned();
    let g = match regime.as_str() {
        "in_memory" => {
            let (off, tgt) = csr_from_rows(input.n, &input.rows);
            Csr::from_vecs(off, tgt)
        }
        "mmap" | "stream" => {
            std::fs::create_dir_all(&work).unwrap_or_else(|e| die(format!("{e}")));
            let files = ingest_files(&input, threads, &work);
            Csr::load(&files, LoadMode::Mmap).unwrap_or_else(|e| die(format!("load: {e}")))
        }
        other => die(format!("unknown regime {other}")),
    };
    let chunks = edge_chunks(&g.offsets, chunk_entries);
    if name == "pagerank" && kernel["weighted"].as_bool() == Some(true) {
        die("the prototype has no weighted PageRank".into());
    }
    let out: Value = match (name.as_str(), regime.as_str()) {
        ("adjacency", _) => json!({ "csr": { "offsets": g.offsets.to_vec(), "targets": g.targets.to_vec() } }),
        ("degree", _) => ints((0..g.n).map(|v| g.deg(v) as u32)),
        ("pagerank", "stream") => {
            let es = EdgeStream::open(&work.join("full").join("targets.u32"), IoMode::Buffered, &g.offsets, &chunks).unwrap_or_else(|e| die(format!("{e}")));
            let (r, _) = extmem::pagerank_semiext::<f64>(&g.offsets, &es, kernel["damping"].as_f64().unwrap_or(0.85), iters, &chunks);
            json!({ "reals": r })
        }
        ("pagerank", _) => {
            let r: Vec<f64> = kernels::pagerank(&g.offsets, &g.targets, kernel["damping"].as_f64().unwrap_or(0.85), iters, &chunks);
            json!({ "reals": r })
        }
        ("wcc", "stream") => {
            let es = EdgeStream::open(&work.join("full").join("targets.u32"), IoMode::Buffered, &g.offsets, &chunks).unwrap_or_else(|e| die(format!("{e}")));
            let comp = extmem::wcc_semiext(&g.offsets, &es, &chunks);
            json!({ "labels": comp.iter().map(|c| c.load(Relaxed)).collect::<Vec<u32>>() })
        }
        ("wcc", _) => {
            let (comp, _) = kernels::wcc_afforest(&g, &chunks);
            json!({ "labels": comp.iter().map(|c| c.load(Relaxed)).collect::<Vec<u32>>() })
        }
        ("bfs", _) => {
            let src = kernel["source"].as_u64().unwrap_or(0) as usize;
            let r = kernels::bfs_do(&g, src, &chunks);
            ints(r.depth.iter().map(|d| d.load(Relaxed)))
        }
        ("core_number", _) => {
            let (core, _, _) = kernels::kcore(&g);
            ints(core.into_iter())
        }
        ("triangle_total", _) => json!({ "count": kernels::triangles(&g).0 }),
        (k, r) => die(format!("kernel {k} is not available in regime {r}")),
    };
    let _ = std::fs::remove_dir_all(&work);
    println!("{}", serde_json::to_string(&out).unwrap());
}
