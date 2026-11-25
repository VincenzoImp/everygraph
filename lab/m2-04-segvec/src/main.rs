// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `segbench`: PageRank (pull, f32, undirected CSR of `glab-csr-v0`) with its arrays in
//! `Vec`s or in `SegVec`s, for issue #31.
//!
//! Two modes, each printing one JSON line on standard output:
//!
//! - `ab`: loads the graph once per layout (`vec`, `seg` = default 1 GiB segments,
//!   `seg-small` = segments of 2^16 elements), then runs the layouts in turn, round
//!   after round, rotating which goes first, so that all see the same machine. Reports
//!   per layout the median seconds per iteration, and the median over rounds of each
//!   layout's time divided by `vec`'s in the same round.
//! - `budget`: every array (and the load buffers) in `SegVec`s under a `Budget` of
//!   `--budget` bytes. The arrays are added up first; if they do not fit, the run is
//!   refused before anything is allocated (`"refused": 1`, exit status 0 so that the
//!   harness records the row as complete and the OOM counters stay meaningful).
//!   Otherwise PageRank runs, and the accountant's peak is reported next to the
//!   cgroup's anonymous peak that the harness measures. `--slack S` sets the budget to
//!   the memory limit read by `everygraph_io::ProcMemoryProbe` minus `S` bytes, so that
//!   a spec states the limit and the slack and the budget follows.
//! - `probe`: prints what `ProcMemoryProbe` reads (no graph needed).
//!
//! Every layout computes the same ranks bit for bit (`checksum`): chunk boundaries and
//! the order of each node's sum do not depend on the layout.

use std::fs::File;
use std::io::Read;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use everygraph_core::{Budget, Memory, MemoryError, MemoryKind, MemoryProbe, SegVec, par};
use everygraph_io::ProcMemoryProbe;

const DAMPING: f32 = 0.85;
const GRAIN: NonZeroUsize = NonZeroUsize::new(4096).unwrap();
const SMALL_SEGMENT_BITS: u32 = 16;
/// Bytes read from a file at a time while loading.
const LOAD_BLOCK: usize = 8 << 20;

/// What the PageRank loop needs from an array.
trait Array<T: Copy>: Sync + Send {
    fn at(&self, i: usize) -> T;
    fn slices(&self, lo: usize, hi: usize, f: impl FnMut(&[T]));
    fn chunks_mut(&mut self, f: impl Fn(usize, &mut [T]) + Send + Sync);
}

impl<T: Copy + Send + Sync> Array<T> for Vec<T> {
    #[inline]
    fn at(&self, i: usize) -> T {
        self[i]
    }
    #[inline]
    fn slices(&self, lo: usize, hi: usize, mut f: impl FnMut(&[T])) {
        f(&self[lo..hi]);
    }
    fn chunks_mut(&mut self, f: impl Fn(usize, &mut [T]) + Send + Sync) {
        par::for_each_chunk_mut(self, GRAIN, f);
    }
}

impl<T: Copy + Send + Sync> Array<T> for SegVec<T> {
    #[inline]
    fn at(&self, i: usize) -> T {
        self[i]
    }
    #[inline]
    fn slices(&self, lo: usize, hi: usize, mut f: impl FnMut(&[T])) {
        for slice in self.range_slices(lo..hi) {
            f(slice);
        }
    }
    fn chunks_mut(&mut self, f: impl Fn(usize, &mut [T]) + Send + Sync) {
        self.for_each_chunk_mut(GRAIN, f);
    }
}

struct Graph<O, T, F> {
    n: usize,
    offsets: O,
    targets: T,
    contrib: F,
    next: F,
}

impl<O: Array<u64>, T: Array<u32>, F: Array<f32>> Graph<O, T, F> {
    fn degree(offsets: &O, v: usize) -> u64 {
        offsets.at(v + 1) - offsets.at(v)
    }

    fn reset(&mut self) {
        let n = self.n;
        let offsets = &self.offsets;
        self.contrib.chunks_mut(|start, chunk| {
            for (k, c) in chunk.iter_mut().enumerate() {
                let d = Self::degree(offsets, start + k);
                *c = if d == 0 {
                    0.0
                } else {
                    1.0 / n as f32 / d as f32
                };
            }
        });
    }

    fn iterate(&mut self) {
        let base = (1.0 - DAMPING) / self.n as f32;
        let (offsets, targets, contrib) = (&self.offsets, &self.targets, &self.contrib);
        self.next.chunks_mut(|start, chunk| {
            for (k, x) in chunk.iter_mut().enumerate() {
                let v = start + k;
                let (lo, hi) = (offsets.at(v) as usize, offsets.at(v + 1) as usize);
                let mut sum = 0.0_f32;
                targets.slices(lo, hi, |slice| {
                    for &u in slice {
                        sum += contrib.at(u as usize);
                    }
                });
                *x = base + DAMPING * sum;
            }
        });
        let next = &self.next;
        self.contrib.chunks_mut(|start, chunk| {
            for (k, c) in chunk.iter_mut().enumerate() {
                let d = Self::degree(offsets, start + k);
                *c = if d == 0 {
                    0.0
                } else {
                    next.at(start + k) / d as f32
                };
            }
        });
    }

    /// Seconds per iteration over `iterations`, from a fresh start.
    fn run(&mut self, iterations: usize) -> f64 {
        self.reset();
        let started = Instant::now();
        for _ in 0..iterations {
            self.iterate();
        }
        started.elapsed().as_secs_f64() / iterations as f64
    }

    fn checksum(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for v in 0..self.n {
            hash ^= u64::from(self.next.at(v).to_bits());
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}

/// `n` and `entries` from `meta.json`.
fn meta(dir: &Path) -> (usize, usize) {
    let text = std::fs::read_to_string(dir.join("meta.json")).expect("meta.json");
    let field = |name: &str| -> usize {
        let at = text.find(&format!("\"{name}\"")).expect(name);
        let rest = &text[at + name.len() + 2..];
        let rest = rest.trim_start_matches([':', ' ']);
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().expect(name)
    };
    (field("n"), field("entries"))
}

/// Reads `path` as little-endian `T`s of `W` bytes, handing them over in blocks.
fn load<const W: usize, T>(
    path: &Path,
    decode: fn([u8; W]) -> T,
    bytes: &mut [u8],
    values: &mut Vec<T>,
    mut sink: impl FnMut(&[T]) -> Result<(), MemoryError>,
) -> Result<(), MemoryError> {
    let mut file = File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    loop {
        let mut filled = 0;
        while filled < bytes.len() {
            match file.read(&mut bytes[filled..]).expect("read") {
                0 => break,
                k => filled += k,
            }
        }
        assert_eq!(filled % W, 0, "{}: truncated", path.display());
        if filled == 0 {
            return Ok(());
        }
        values.clear();
        values.extend(
            bytes[..filled]
                .chunks_exact(W)
                .map(|c| decode(c.try_into().unwrap())),
        );
        sink(values)?;
    }
}

fn load_vec(dir: &Path, n: usize, entries: usize) -> Graph<Vec<u64>, Vec<u32>, Vec<f32>> {
    let mut bytes = vec![0_u8; LOAD_BLOCK];
    let mut offsets = Vec::with_capacity(n + 1);
    let mut targets = Vec::with_capacity(entries);
    let mut buffer = Vec::new();
    load(
        &dir.join("offsets.u64"),
        u64::from_le_bytes,
        &mut bytes,
        &mut buffer,
        |b| {
            offsets.extend_from_slice(b);
            Ok(())
        },
    )
    .unwrap();
    let mut buffer = Vec::new();
    load(
        &dir.join("targets.u32"),
        u32::from_le_bytes,
        &mut bytes,
        &mut buffer,
        |b| {
            targets.extend_from_slice(b);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!((offsets.len(), targets.len()), (n + 1, entries));
    Graph {
        n,
        offsets,
        targets,
        contrib: vec![0.0; n],
        next: vec![0.0; n],
    }
}

fn new_seg<T: Copy>(memory: &Memory, purpose: &'static str, bits: Option<u32>) -> SegVec<T> {
    match bits {
        Some(bits) => SegVec::with_segment_bits(memory, purpose, bits),
        None => SegVec::new(memory, purpose),
    }
}

type SegGraph = Graph<SegVec<u64>, SegVec<u32>, SegVec<f32>>;

fn load_seg(
    memory: &Memory,
    dir: &Path,
    n: usize,
    entries: usize,
    bits: Option<u32>,
) -> Result<SegGraph, MemoryError> {
    // The load buffers are counted too: they are anonymous memory like the rest.
    let mut bytes = SegVec::try_with_len(memory, "load buffer", LOAD_BLOCK, 0_u8)?;
    let bytes = bytes.as_mut_slice().expect("one segment");
    let _decoded = memory.reserve("decoded block", MemoryKind::Anonymous, LOAD_BLOCK as u64)?;

    // One decode buffer at a time (LOAD_BLOCK bytes, reserved above as "decoded block").
    let mut offsets: SegVec<u64> = new_seg(memory, "offsets", bits);
    offsets.try_reserve(n + 1)?;
    {
        let mut buffer = Vec::with_capacity(LOAD_BLOCK / 8);
        load(
            &dir.join("offsets.u64"),
            u64::from_le_bytes,
            bytes,
            &mut buffer,
            |b| offsets.try_extend_from_slice(b),
        )?;
    }
    let mut targets: SegVec<u32> = new_seg(memory, "targets", bits);
    targets.try_reserve(entries)?;
    {
        let mut buffer = Vec::with_capacity(LOAD_BLOCK / 4);
        load(
            &dir.join("targets.u32"),
            u32::from_le_bytes,
            bytes,
            &mut buffer,
            |b| targets.try_extend_from_slice(b),
        )?;
    }
    assert_eq!((offsets.len(), targets.len()), (n + 1, entries));
    let mut contrib: SegVec<f32> = new_seg(memory, "contributions", bits);
    contrib.try_resize(n, 0.0)?;
    let mut next: SegVec<f32> = new_seg(memory, "ranks", bits);
    next.try_resize(n, 0.0)?;
    Ok(Graph {
        n,
        offsets,
        targets,
        contrib,
        next,
    })
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}

enum Loaded {
    Vec(Graph<Vec<u64>, Vec<u32>, Vec<f32>>),
    Seg(SegGraph),
}

impl Loaded {
    fn run(&mut self, iterations: usize) -> f64 {
        match self {
            Self::Vec(g) => g.run(iterations),
            Self::Seg(g) => g.run(iterations),
        }
    }
    fn checksum(&self) -> u64 {
        match self {
            Self::Vec(g) => g.checksum(),
            Self::Seg(g) => g.checksum(),
        }
    }
}

struct Args {
    mode: String,
    graph: PathBuf,
    layouts: Vec<String>,
    rounds: usize,
    iterations: usize,
    threads: usize,
    budget: u64,
    slack: Option<u64>,
}

fn args() -> Args {
    let mut args = Args {
        mode: String::new(),
        graph: PathBuf::new(),
        layouts: vec!["vec".into(), "seg".into(), "seg-small".into()],
        rounds: 5,
        iterations: 10,
        threads: 16,
        budget: u64::MAX,
        slack: None,
    };
    let mut it = std::env::args().skip(1);
    args.mode = it
        .next()
        .expect("usage: segbench ab|budget --graph DIR [options]");
    while let Some(flag) = it.next() {
        let value = it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--graph" => args.graph = value.into(),
            "--layouts" => args.layouts = value.split(',').map(String::from).collect(),
            "--rounds" => args.rounds = value.parse().unwrap(),
            "--iterations" => args.iterations = value.parse().unwrap(),
            "--threads" => args.threads = value.parse().unwrap(),
            "--budget" => args.budget = value.parse().unwrap(),
            "--slack" => args.slack = Some(value.parse().unwrap()),
            _ => panic!("unknown option {flag}"),
        }
    }
    args
}

fn main() {
    let args = args();
    if args.mode == "probe" {
        // What ProcMemoryProbe reads here (the review's live check of #154).
        let m = ProcMemoryProbe::new().probe().expect("memory probe");
        println!(
            "{{\"machine_total\":{},\"machine_available\":{},\"limit\":{:?},\"limit_available\":{:?},\"total\":{},\"available\":{}}}",
            m.machine_total,
            m.machine_available,
            m.limit,
            m.limit_available,
            m.total(),
            m.available()
        );
        return;
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(args.threads)
        .build_global()
        .unwrap();
    let (n, entries) = meta(&args.graph);
    match args.mode.as_str() {
        "ab" => ab(&args, n, entries),
        "budget" => budget(&args, n, entries),
        other => panic!("unknown mode {other}"),
    }
}

fn ab(args: &Args, n: usize, entries: usize) {
    let memory = Memory::unlimited();
    let mut loaded: Vec<Loaded> = args
        .layouts
        .iter()
        .map(|layout| match layout.as_str() {
            "vec" => Loaded::Vec(load_vec(&args.graph, n, entries)),
            "seg" => Loaded::Seg(load_seg(&memory, &args.graph, n, entries, None).unwrap()),
            "seg-small" => Loaded::Seg(
                load_seg(&memory, &args.graph, n, entries, Some(SMALL_SEGMENT_BITS)).unwrap(),
            ),
            other => panic!("unknown layout {other}"),
        })
        .collect();
    let layouts = loaded.len();
    let mut times = vec![Vec::new(); layouts];
    // Warm-up: one untimed run of each.
    for graph in &mut loaded {
        graph.run(1);
    }
    for round in 0..args.rounds {
        for k in 0..layouts {
            let which = (round + k) % layouts;
            times[which].push(loaded[which].run(args.iterations));
        }
    }
    let checksums: Vec<u64> = loaded.iter().map(Loaded::checksum).collect();
    let mut fields = vec![
        format!("\"graph\":{:?}", args.graph.display().to_string()),
        format!("\"n\":{n},\"entries\":{entries}"),
        format!(
            "\"threads\":{},\"rounds\":{},\"iterations\":{}",
            args.threads, args.rounds, args.iterations
        ),
        format!(
            "\"same_ranks\":{}",
            u8::from(checksums.windows(2).all(|w| w[0] == w[1]))
        ),
        format!("\"checksum\":\"{:016x}\"", checksums[0]),
    ];
    for (layout, samples) in args.layouts.iter().zip(&times) {
        let key = layout.replace('-', "_");
        fields.push(format!(
            "\"{key}_s_per_iter\":{:.6}",
            median(&mut samples.clone())
        ));
        if let Some(base) = args.layouts.iter().position(|l| l == "vec") {
            let mut ratios: Vec<f64> = samples
                .iter()
                .zip(&times[base])
                .map(|(a, b)| a / b)
                .collect();
            fields.push(format!("\"{key}_over_vec\":{:.4}", median(&mut ratios)));
        }
    }
    println!("{{{}}}", fields.join(","));
}

fn budget(args: &Args, n: usize, entries: usize) {
    let system = ProcMemoryProbe::new().probe().expect("memory probe");
    let limit = system.total();
    let budget_bytes = match args.slack {
        Some(slack) => limit.saturating_sub(slack),
        None => args.budget,
    };
    let budget = Arc::new(Budget::new(budget_bytes));
    let memory = Memory::from(Arc::clone(&budget));
    // The plan: everything this run will hold, added up before anything is allocated.
    let parts = [
        ("offsets", SegVec::<u64>::bytes_for(n as u64 + 1)),
        ("targets", SegVec::<u32>::bytes_for(entries as u64)),
        ("contributions", SegVec::<f32>::bytes_for(n as u64)),
        ("ranks", SegVec::<f32>::bytes_for(n as u64)),
        ("load buffers", Some(2 * LOAD_BLOCK as u64)),
    ];
    let needed: u64 = parts.iter().map(|(_, b)| b.unwrap()).sum();
    let available = memory.available();
    if needed > available {
        println!(
            "{{\"refused\":1,\"needed_bytes\":{needed},\"available_bytes\":{available},\"budget_bytes\":{budget_bytes},\"limit_bytes\":{limit},\"accountant_peak_bytes\":{}}}",
            budget.peak()
        );
        return;
    }
    let mut graph =
        load_seg(&memory, &args.graph, n, entries, None).expect("fits: the plan said so");
    let seconds = graph.run(args.iterations);
    let report = budget.report();
    let purposes: Vec<String> = report
        .purposes
        .iter()
        .map(|p| format!("{:?}:{}", p.purpose, p.peak))
        .collect();
    println!(
        "{{\"refused\":0,\"needed_bytes\":{needed},\"budget_bytes\":{budget_bytes},\"limit_bytes\":{limit},\"accountant_peak_bytes\":{},\"s_per_iter\":{seconds:.6},\"checksum\":\"{:016x}\",\"peak_by_purpose\":{{{}}}}}",
        report.peak,
        graph.checksum(),
        purposes.join(",")
    );
}
