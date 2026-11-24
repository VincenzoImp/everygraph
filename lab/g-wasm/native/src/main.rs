//! Native driver for graphcore. Usage:
//!   gnative text <edge-list.txt>      parse -> CSR -> PR(20) -> CC
//!   gnative csr  <glab-csr-v0 dir>    load CSR -> PR(20) -> CC
//!   gnative gen  <m_edges> <n_nodes>  synthetic -> CSR -> PR -> CC
//! Prints one JSON object. Threads: RAYON_NUM_THREADS.
use graphcore::*;
use std::time::Instant;

fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

fn read_vec<T: Copy>(path: &str) -> Vec<T> {
    let bytes = std::fs::read(path).expect(path);
    let sz = std::mem::size_of::<T>();
    let n = bytes.len() / sz;
    let mut v: Vec<T> = Vec::with_capacity(n);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), v.as_mut_ptr() as *mut u8, n * sz);
        v.set_len(n);
    }
    v
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mode = a.get(1).map(|s| s.as_str()).unwrap_or("");
    let mut out = vec![("threads".to_string(), par::threads() as f64)];
    let mut put = |k: &str, v: f64| out.push((k.to_string(), v));
    let g = match mode {
        "text" => {
            let t = Instant::now();
            let buf = std::fs::read(&a[2]).unwrap();
            put("read_s", secs(t));
            put("text_bytes", buf.len() as f64);
            let t = Instant::now();
            let el = parse_edges(&buf);
            put("parse_s", secs(t));
            put("edges_in", el.len() as f64);
            drop(buf);
            let t = Instant::now();
            let (g, _ids) = build_csr_undirected(el);
            put("csr_s", secs(t));
            g
        }
        "csr" => {
            let t = Instant::now();
            let offsets: Vec<u64> = read_vec(&format!("{}/offsets.u64", a[2]));
            let targets: Vec<u32> = read_vec(&format!("{}/targets.u32", a[2]));
            put("load_s", secs(t));
            Csr::from_u64_offsets(&offsets, targets)
        }
        "text2" => {
            // two-pass streaming from disk in 16 MiB chunks (what the browser does with a File)
            use std::io::Read;
            let t = Instant::now();
            let mut tp = TextTwoPass::new();
            let mut buf = vec![0u8; 16 << 20];
            for pass in 0..2 {
                let mut f = std::fs::File::open(&a[2]).unwrap();
                loop {
                    let k = f.read(&mut buf).unwrap();
                    if k == 0 { break; }
                    tp.push(&buf[..k]);
                }
                if pass == 0 { put("pass1_s", secs(t)); tp.next_pass(); }
            }
            put("edges_in", tp.edges_seen as f64);
            let (g, _) = tp.finish();
            put("ingest_two_pass_s", secs(t));
            g
        }
        "gen" => {
            let m: usize = a[2].parse().unwrap();
            let n: u32 = a[3].parse().unwrap();
            let t = Instant::now();
            let el = gen_edges(n, m, 42);
            put("gen_s", secs(t));
            let t = Instant::now();
            let (g, _) = build_csr_undirected(el);
            put("csr_s", secs(t));
            g
        }
        _ => {
            eprintln!("usage: gnative text|csr|gen ...");
            std::process::exit(2)
        }
    };
    put("n", g.n() as f64);
    put("entries", g.entries() as f64);
    let t = Instant::now();
    let pr = pagerank(&g, 20, 0.85);
    put("pr_s", secs(t));
    let (s, mx) = pr_checksum(&pr);
    put("pr_sum", s);
    put("pr_max", mx);
    drop(pr);
    let t = Instant::now();
    let cc = connected_components(&g);
    put("cc_s", secs(t));
    let (nc, lg) = cc_stats(&cc);
    put("cc_count", nc as f64);
    put("cc_largest", lg as f64);
    let body: Vec<String> = out.iter().map(|(k, v)| format!("\"{}\":{}", k, v)).collect();
    println!("{{\"mode\":\"{}\",{}}}", mode, body.join(","));
}
