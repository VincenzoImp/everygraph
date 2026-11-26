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
//! encode`, memory-mapped, instead of the CSR in the heap.
//!
//! Prints one JSON line: the times, the fan-out distribution, the cover and the super
//! level, the kinds, the modularity of the partition and of the drawn top level, and a
//! hash of the tree (the same at every thread count). `--store FILE` writes the graph
//! and the map tree into a new graph file, checks it with `verify --deep` and reports
//! the result's bytes and its levels. `--export DIR` writes the tree as the hierarchy
//! of the lab layout (`lab/e-map/gmap layout --hier DIR`).

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

use everygraph_community::hierarchy;
use everygraph_community::leiden;
use everygraph_community::map::{self, Kind, MapOptions, MapTree, StoredMap, SuperLevel};
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

/// Writes the graph (its adjacency `index` and `data`, read as `source`) and the tree
/// into a new graph file, and checks it.
fn store<S: Source>(
    path: &Path,
    index: &[u8],
    data: &[u8],
    source: &S,
    o: &MapOptions,
    t: &MapTree,
) -> String {
    let memory = Memory::unlimited();
    let start = Instant::now();
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
        ("adjacency.out.index", Section::ADJACENCY_INDEX, index),
        ("adjacency.out.data", Section::ADJACENCY_DATA, data),
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
    map::store(
        &mut w,
        &key,
        &about,
        source,
        &memory,
        Settings::default(),
        t,
    )
    .expect("stored");
    w.commit().expect("commit");
    drop(w);
    let write = written.elapsed().as_secs_f64();
    let mapped = MmapSource::open(path).expect("map");
    let file = Container::open(&mapped).expect("a container");
    let bytes = file.results().expect("results")[0].bytes;
    let m = StoredMap::find(&file, &key).expect("read").expect("found");
    let siblings = m.siblings().expect("siblings").index().entries();
    let regions = m.regions();
    let checked = Instant::now();
    let mut report = verify(&mapped, Depth::Deep, &memory).expect("verify");
    hierarchy::check(&file, &memory, &mut report).expect("check");
    format!(
        ", \"store_seconds\": {write}, \"map_bytes\": {bytes}, \"graph_bytes\": {}, \
         \"stored_regions\": {regions}, \"sibling_entries\": {siblings}, \
         \"verify_seconds\": {}, \"problems\": {}, \"unchecked\": {}, \
         \"total_store_seconds\": {}",
        index.len() + data.len(),
        checked.elapsed().as_secs_f64(),
        report.problems.len(),
        report.unchecked.len(),
        start.elapsed().as_secs_f64()
    )
}

/// Writes the tree as the lab layout's hierarchy (`lab/e-map/gmap`, as
/// `lab/m1-05-hierarchy-shaping/shape.py export` writes it): level `k` holds the regions
/// of height `k`, each repeated up to the level below its parent; `level_0.u32` maps the
/// nodes, `level_k.u32` the units of level `k − 1`; `level_k.kind.u8` is 1 for a
/// community or super region, 2 for an aggregate, 3 for a subcommunity; and
/// `level_k.count.u32` the communities under an aggregate.
fn export(dir: &Path, t: &MapTree) {
    std::fs::create_dir_all(dir).expect("export directory");
    let r = t.parent.len();
    let parent = |x: usize| t.parent[x] as usize;
    let mut height = vec![0_usize; r];
    let mut changed = true;
    while changed {
        changed = false;
        for x in 1..r {
            if height[parent(x)] < height[x] + 1 {
                height[parent(x)] = height[x] + 1;
                changed = true;
            }
        }
    }
    let mut count = vec![0_u32; r];
    for x in (1..r).filter(|&x| t.kind[x] != Kind::Aggregate) {
        let mut p = parent(x);
        while p > 0 && t.kind[p] == Kind::Aggregate {
            count[p] += 1;
            p = parent(p);
        }
    }
    let lab = |k: Kind| match k {
        Kind::Aggregate => 2_u8,
        Kind::Subcommunity => 3,
        _ => 1,
    };
    let write = |name: String, bytes: Vec<u8>| {
        std::fs::write(dir.join(name), bytes).expect("export");
    };
    let u32s = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let mut unit = vec![u32::MAX; r];
    let mut levels = Vec::new();
    let mut below: Vec<usize> = Vec::new();
    for k in 0..height[0] {
        let here: Vec<usize> = (1..r)
            .filter(|&x| height[x] <= k && k < height[parent(x)])
            .collect();
        for (i, &x) in here.iter().enumerate() {
            unit[x] = i as u32;
        }
        let map: Vec<u32> = if k == 0 {
            t.leaf.iter().map(|&l| unit[l as usize]).collect()
        } else {
            let up = |x: usize| if k < height[parent(x)] { x } else { parent(x) };
            below.iter().map(|&x| unit[up(x)]).collect()
        };
        write(format!("level_{k}.u32"), u32s(&map));
        write(
            format!("level_{k}.kind.u8"),
            here.iter().map(|&x| lab(t.kind[x])).collect(),
        );
        let counts: Vec<u32> = here.iter().map(|&x| count[x]).collect();
        write(format!("level_{k}.count.u32"), u32s(&counts));
        levels.push(here.len().to_string());
        below = here;
    }
    let meta = format!(
        "{{\"n\": {}, \"levels\": [{}]}}",
        t.leaf.len(),
        levels.join(", ")
    );
    write("hier.json".into(), meta.into_bytes());
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
        c.modularity,
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
            let bytes = std::fs::read(dir.join("adjacency.index")).expect("the index");
            let index = AdjacencyIndex::parse(&bytes).expect("an index");
            let data = MmapSource::open(dir.join("adjacency.data")).expect("the data");
            let source = Compressed::mapped(&index, &data).expect("the data part");
            let (line, t) = shape(&source, &o, threads);
            if let Some(d) = a.get("export") {
                export(Path::new(d), &t);
            }
            let stored = a.get("store").map_or_else(String::new, |p| {
                let all = everygraph_core::ByteSource::as_slice(&data).expect("mapped");
                store(Path::new(p), &bytes, all, &source, &o, &t)
            });
            return format!("{line}{stored}");
        }
        let offsets = read_words(&graph.join("offsets.u64"), u64::from_le_bytes);
        let targets = read_words(&graph.join("targets.u32"), u32::from_le_bytes);
        let csr = Csr::new(&offsets, &targets).expect("a CSR");
        let (line, t) = shape(&csr, &o, threads);
        if let Some(d) = a.get("export") {
            export(Path::new(d), &t);
        }
        let stored = a.get("store").map_or_else(String::new, |p| {
            let encoded = encode(&offsets, &targets).expect("encode");
            store(Path::new(p), &encoded.index, &encoded.data, &csr, &o, &t)
        });
        format!("{line}{stored}")
    })
    .expect("a thread pool");
    println!(
        "{{\"graph\": \"{}\", {line}}}",
        graph.file_name().unwrap_or_default().to_string_lossy()
    );
}
