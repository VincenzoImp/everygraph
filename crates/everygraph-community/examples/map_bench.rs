// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The map tree (#63) on a CSR directory of the dataset registry (`offsets.u64`,
//! `targets.u32`): Leiden, then the shaping, timed inside the process.
//!
//! ```text
//! map_bench --graph DIR [--mmap STORE] [--threads T] [--fan-out F] [--leaf L]
//!     [--super auto|always|never] [--store FILE] [--export DIR]
//! ```
//!
//! `--mmap STORE` reads the graph from a compressed store written by `leiden_bench
//! encode`, memory-mapped, instead of the CSR in the heap (`--store` then does nothing).
//!
//! Prints one JSON line: the times, the fan-out distribution, the cover and the super
//! level, the kinds, the modularity of the partition and of the drawn top level, and a
//! hash of the tree (the same at every thread count). `--store FILE` writes the graph
//! and the map tree into a new graph file, checks it with `verify --deep` and reports
//! the result's bytes and its levels. `--export DIR` writes the tree as the lab's
//! `tree_rec.npz` arrays (`par`, `kind`, `nn`, `leaf` as raw little-endian files), for
//! the lab layout and metrics.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use everygraph_community::hierarchy::{self, Hierarchy};
use everygraph_community::leiden;
use everygraph_community::map::{self, Kind, MapOptions, MapTree, SuperLevel};
use everygraph_core::{Budget, Memory, par};
use everygraph_format::adjacency::{AdjacencyIndex, encode};
use everygraph_format::container::{Container, Kind as Section, Provenance, Writer};
use everygraph_format::verify::{Depth, verify};
use everygraph_io::{FileSink, MmapSource};
use everygraph_kernels::edge_map::{Compressed, Csr, Settings, Source};

fn read_words<const W: usize, T>(path: &Path, from: fn([u8; W]) -> T) -> Vec<T> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    bytes
        .chunks_exact(W)
        .map(|c| from(c.try_into().expect("W bytes")))
        .collect()
}

fn fnv(h: u64, words: impl Iterator<Item = u32>) -> u64 {
    words.fold(h, |h, w| {
        (h ^ u64::from(w)).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

fn code(k: Kind) -> u8 {
    match k {
        Kind::Root => 0,
        Kind::Super => 1,
        Kind::Community => 2,
        Kind::Subcommunity => 3,
        Kind::Aggregate => 4,
    }
}

fn hash(t: &MapTree) -> u64 {
    let h = fnv(0xCBF2_9CE4_8422_2325, t.parent.iter().copied());
    let h = fnv(h, t.kind.iter().map(|&k| u32::from(code(k))));
    fnv(h, t.leaf.iter().copied())
}

/// Writes the graph and the tree into a new graph file, and checks it.
fn store(path: &Path, offsets: &[u64], targets: &[u32], o: &MapOptions, t: &MapTree) -> String {
    let memory = Memory::unlimited();
    let csr = Csr::new(offsets, targets).expect("a CSR");
    let start = Instant::now();
    let encoded = encode(offsets, targets).expect("encode");
    let mut w = Writer::create(FileSink::create(path).expect("the file")).expect("a container");
    let record = Provenance {
        engine: "map_bench".into(),
        operation: "ingest".into(),
        parameters: [("directed", "false"), ("self_loops", "keep")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        ..Provenance::default()
    };
    w.add_provenance("provenance/ingest", &record)
        .expect("record");
    for (name, kind, bytes) in [
        (
            "adjacency.out.index",
            Section::ADJACENCY_INDEX,
            &encoded.index,
        ),
        ("adjacency.out.data", Section::ADJACENCY_DATA, &encoded.data),
    ] {
        w.add_section(name, kind, bytes, Some("provenance/ingest"))
            .expect("section");
    }
    w.commit().expect("commit");
    let key = map::key(w.graph_identity(), o).expect("a key");
    let about = Provenance {
        engine: "map_bench".into(),
        ..Provenance::default()
    };
    let written = Instant::now();
    map::store(&mut w, &key, &about, &csr, &memory, Settings::default(), t).expect("stored");
    w.commit().expect("commit");
    drop(w);
    let write = written.elapsed().as_secs_f64();
    let mapped = MmapSource::open(path).expect("map");
    let file = Container::open(&mapped).expect("a container");
    let bytes = file.results().expect("results")[0].bytes;
    let h = Hierarchy::find(&file, &key).expect("read").expect("found");
    let levels: Vec<String> = (1..=h.levels())
        .map(|l| h.count(l).expect("a level").to_string())
        .collect();
    let checked = Instant::now();
    let mut report = verify(&mapped, Depth::Deep, &memory).expect("verify");
    hierarchy::check(&file, &memory, &mut report).expect("check");
    format!(
        ", \"store_seconds\": {write}, \"map_bytes\": {bytes}, \"graph_bytes\": {}, \
         \"stored_levels\": [{}], \"verify_seconds\": {}, \"problems\": {}, \"unchecked\": {}, \
         \"total_store_seconds\": {}",
        encoded.data.len() + encoded.index.len(),
        levels.join(", "),
        checked.elapsed().as_secs_f64(),
        report.problems.len(),
        report.unchecked.len(),
        start.elapsed().as_secs_f64()
    )
}

fn export(dir: &Path, t: &MapTree) {
    std::fs::create_dir_all(dir).expect("export directory");
    let u32s = |name: &str, values: &mut dyn Iterator<Item = u32>| {
        let bytes: Vec<u8> = values.flat_map(u32::to_le_bytes).collect();
        std::fs::write(dir.join(name), bytes).expect("export");
    };
    u32s("par.u32", &mut t.parent.iter().copied());
    u32s("nn.u32", &mut t.nodes.iter().copied());
    u32s("leaf.u32", &mut t.leaf.iter().copied());
    let kinds: Vec<u8> = t.kind.iter().map(|&k| code(k)).collect();
    std::fs::write(dir.join("kind.u8"), kinds).expect("export");
}

/// Leiden and the map tree on `source`: the JSON fields, and the tree.
fn shape<S: Source>(source: &S, o: &MapOptions, threads: NonZeroUsize) -> (String, MapTree) {
    let budget = Arc::new(Budget::unlimited());
    let memory = Memory::new(budget.clone());
    let settings = Settings::default();
    let t0 = Instant::now();
    let c = leiden(source, &memory, settings, &o.leiden).expect("leiden");
    let leiden_s = t0.elapsed().as_secs_f64();
    let leiden_peak = budget.peak();
    let t1 = Instant::now();
    let t = map::build(source, &memory, settings, &c.membership, o).expect("map");
    let map_s = t1.elapsed().as_secs_f64();
    let s = &t.shape;
    let mut kinds = [0_u32; 5];
    for &k in t.kind.iter() {
        kinds[code(k) as usize] += 1;
    }
    let line = format!(
        "\"threads\": {threads}, \"leiden_seconds\": {leiden_s}, \
         \"map_seconds\": {map_s}, \"leiden_peak_reserved\": {leiden_peak}, \
         \"peak_reserved\": {}, \"communities\": {}, \"regions\": {}, \
         \"kinds\": {{\"super\": {}, \"community\": {}, \"subcommunity\": {}, \
         \"aggregate\": {}}}, \"parents\": {}, \"fan_out_median\": {}, \"fan_out_p95\": {}, \
         \"fan_out_max\": {}, \"share_4_64\": {}, \"share_target\": {}, \"share_2_3\": {}, \
         \"depth\": {}, \"cover\": {}, \"super_resolution\": {}, \"rounds\": {}, \
         \"unsplit_leaves\": {}, \"unsplit_nodes\": {}, \"share_under_aggregate\": {}, \
         \"q_partition\": {}, \"q_top\": {}, \"hash\": \"{:016x}\"",
        budget.peak(),
        c.count,
        t.parent.len(),
        kinds[1],
        kinds[2],
        kinds[3],
        kinds[4],
        s.fan_out.len(),
        s.quantile(0.5),
        s.quantile(0.95),
        s.quantile(1.0),
        s.share(4..=64),
        s.share(s.target.clone()),
        s.share(2..=3),
        s.depth,
        s.cover,
        s.super_resolution.map_or("null".into(), |r| r.to_string()),
        s.rounds,
        s.unsplit.0,
        s.unsplit.1,
        s.under_aggregate as f64 / t.leaf.len() as f64,
        s.partition_modularity,
        s.top_modularity,
        hash(&t),
    );
    (line, t)
}

fn main() {
    let mut it = std::env::args().skip(1);
    let mut a: HashMap<String, String> = HashMap::new();
    while let Some(k) = it.next() {
        a.insert(
            k.trim_start_matches("--").to_owned(),
            it.next().unwrap_or_default(),
        );
    }
    let graph = PathBuf::from(&a["graph"]);
    let threads: usize = a
        .get("threads")
        .map_or(1, |s| s.parse().expect("--threads"));
    let threads = NonZeroUsize::new(threads).expect("--threads > 0");
    let mut o = MapOptions::default();
    if let Some(f) = a.get("fan-out") {
        o.fan_out = 8..=f.parse().expect("--fan-out");
    }
    if let Some(l) = a.get("leaf") {
        o.leaf = l.parse().expect("--leaf");
    }
    match a.get("super").map(String::as_str) {
        Some("always") => o.super_level = SuperLevel::Always,
        Some("never") => o.super_level = SuperLevel::Never,
        _ => {}
    }
    let line = par::with_threads(threads, || {
        if let Some(dir) = a.get("mmap") {
            // A store written by `leiden_bench encode`, memory-mapped.
            let dir = Path::new(dir);
            let index = std::fs::read(dir.join("adjacency.index")).expect("the index");
            let index = AdjacencyIndex::parse(&index).expect("an index");
            let data = MmapSource::open(dir.join("adjacency.data")).expect("the data");
            let source = Compressed::mapped(&index, &data).expect("the data part");
            let (line, t) = shape(&source, &o, threads);
            if let Some(d) = a.get("export") {
                export(Path::new(d), &t);
            }
            return line;
        }
        let offsets = read_words(&graph.join("offsets.u64"), u64::from_le_bytes);
        let targets = read_words(&graph.join("targets.u32"), u32::from_le_bytes);
        let csr = Csr::new(&offsets, &targets).expect("a CSR");
        let (line, t) = shape(&csr, &o, threads);
        if let Some(d) = a.get("export") {
            export(Path::new(d), &t);
        }
        let stored = a.get("store").map_or_else(String::new, |p| {
            store(Path::new(p), &offsets, &targets, &o, &t)
        });
        format!("{line}{stored}")
    })
    .expect("a thread pool");
    println!(
        "{{\"graph\": \"{}\", {line}}}",
        graph.file_name().unwrap_or_default().to_string_lossy()
    );
}
