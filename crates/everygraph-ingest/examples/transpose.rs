// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The in-adjacency of a large directed graph (issue #38), added to a graph file that
//! holds only its out-adjacency.
//!
//! ```text
//! transpose prepare --csr DIR --out BASE.eg
//! transpose run --base BASE.eg --work FILE.eg --budget 24G [--check DIR]
//! ```
//!
//! `prepare` encodes a raw CSR (`offsets.u64`, `targets.u32`, lists sorted and without
//! repeats, as `datasets/` stores twitter-2010) into a graph file with the out-adjacency
//! and the provenance of a directed build: no dictionary, which the transposition does
//! not read. `run` copies `BASE.eg` to `FILE.eg` (not timed), then times
//! `add_in_adjacency` on it, with `fsync`, and prints one JSON line: the seconds, the
//! bytes this process wrote (`/proc/self/io` `write_bytes`), the sizes of the sections.
//! `--check DIR` then compares the in-lists with the raw CSR of the transpose in `DIR`.
//!
//! Threads: `RAYON_NUM_THREADS`. Not a measurement for the record: the harness runs
//! those (`bench/specs/transpose.toml`).

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a benchmark binary reports on its streams"
)]

#[cfg(target_os = "linux")]
mod bench {
    use std::fs::File;
    use std::io::{self, Read};
    use std::num::NonZeroUsize;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    use everygraph_core::{Budget, ByteSource, Memory, NodeId};
    use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, Batch, encode};
    use everygraph_format::container::{Container, Kind, Provenance, Writer};
    use everygraph_ingest::build::{
        IN_DATA, IN_INDEX, OUT_DATA, OUT_INDEX, PROVENANCE, Silent, add_in_adjacency,
    };
    use everygraph_ingest::ids::Scratch;
    use everygraph_io::{FileSink, MmapSource};

    pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync>;

    /// Scratch files in a directory, unlinked at once and mapped once sealed.
    struct DirScratch {
        dir: PathBuf,
        next: AtomicU64,
    }

    impl Scratch for DirScratch {
        type Source = MmapSource;
        type Writer = File;

        fn create(&self, purpose: &'static str) -> io::Result<File> {
            let i = self.next.fetch_add(1, Ordering::Relaxed);
            let path = self
                .dir
                .join(format!("{i:08}-{}", purpose.replace(' ', "-")));
            let file = File::options()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)?;
            std::fs::remove_file(&path)?;
            Ok(file)
        }

        fn seal(&self, file: File) -> io::Result<MmapSource> {
            MmapSource::from_file(&file)
        }
    }

    fn write_bytes() -> u64 {
        std::fs::read_to_string("/proc/self/io")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("write_bytes: ").map(str::to_owned))
            })
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0)
    }

    fn parse_bytes(s: &str) -> Result<u64, BoxError> {
        let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
        let shift = match unit {
            "" => 0,
            "M" => 20,
            "G" => 30,
            _ => return Err(format!("bad size {s}").into()),
        };
        Ok(num.parse::<u64>()? << shift)
    }

    /// A little-endian array file of `W`-byte words, read in 64 MiB pieces.
    fn words<const W: usize, T>(path: &Path, word: fn([u8; W]) -> T) -> Result<Vec<T>, BoxError> {
        let mut file = File::open(path)?;
        let len = usize::try_from(file.metadata()?.len())? / W;
        let mut out = Vec::with_capacity(len);
        let mut buf = vec![0_u8; W << 24];
        while out.len() < len {
            let n = (len - out.len()).min(1 << 24) * W;
            file.read_exact(&mut buf[..n])?;
            out.extend(
                buf[..n]
                    .chunks_exact(W)
                    .map(|c| word(c.try_into().expect("W bytes"))),
            );
        }
        Ok(out)
    }

    fn csr(dir: &Path) -> Result<(Vec<u64>, Vec<u32>), BoxError> {
        Ok((
            words(&dir.join("offsets.u64"), u64::from_le_bytes)?,
            words(&dir.join("targets.u32"), u32::from_le_bytes)?,
        ))
    }

    fn prepare(dir: &Path, out: &Path) -> Result<(), BoxError> {
        let (offsets, targets) = csr(dir)?;
        let encoded = encode(&offsets, &targets)?;
        drop((offsets, targets));
        let mut writer = Writer::create(FileSink::create(out)?)?;
        let parameters = [
            ("directed", "true"),
            ("self_loops", "keep"),
            ("duplicates", "collapse"),
        ];
        let record = Provenance {
            engine: "everygraph transpose example".into(),
            operation: "ingest".into(),
            parameters: parameters
                .iter()
                .map(|&(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
            ..Provenance::default()
        };
        writer.add_provenance(PROVENANCE, &record)?;
        writer.add_section(
            OUT_INDEX,
            Kind::ADJACENCY_INDEX,
            &encoded.index,
            Some(PROVENANCE),
        )?;
        writer.add_section(
            OUT_DATA,
            Kind::ADJACENCY_DATA,
            &encoded.data,
            Some(PROVENANCE),
        )?;
        writer.commit()?;
        Ok(())
    }

    /// Compares the in-adjacency of `file` with the raw CSR in `dir`.
    fn compare(file: &Path, dir: &Path) -> Result<(), BoxError> {
        let (offsets, targets) = csr(dir)?;
        let container = Container::open(MmapSource::open(file)?)?;
        let section = |name: &str| -> Result<&[u8], BoxError> {
            let e = container.get(name).ok_or("no such section")?;
            Ok(container
                .source()
                .slice(e.offset..e.offset + e.len)
                .ok_or("not mapped")?)
        };
        let index = AdjacencyIndex::parse(section(IN_INDEX)?)?;
        let adjacency = Adjacency::new(&index, section(IN_DATA)?)?;
        if u64::from(index.nodes()) + 1 != offsets.len() as u64 {
            return Err("the node counts differ".into());
        }
        let mut batch = Batch::default();
        for b in 0..index.blocks() {
            let nodes = index.block_nodes(b);
            adjacency.decode_nodes(nodes.clone(), usize::MAX, &mut batch)?;
            let from = usize::try_from(offsets[usize::try_from(nodes.start)?])?;
            let to = usize::try_from(offsets[usize::try_from(nodes.end)?])?;
            let entry = index.entry_offset(NodeId::new(nodes.start)).get();
            if batch.targets() != &targets[from..to] || entry != from as u64 {
                return Err(format!("the in-lists of block {b} differ").into());
            }
        }
        Ok(())
    }

    fn run(base: &Path, work: &Path, budget: u64, check: Option<&Path>) -> Result<(), BoxError> {
        std::fs::copy(base, work)?;
        let scratch_dir = work.with_extension("scratch");
        std::fs::create_dir_all(&scratch_dir)?;
        let scratch = DirScratch {
            dir: scratch_dir.clone(),
            next: AtomicU64::new(0),
        };
        let threads = std::env::var("RAYON_NUM_THREADS")
            .ok()
            .and_then(|t| t.parse().ok())
            .and_then(NonZeroUsize::new)
            .unwrap_or(NonZeroUsize::MIN);
        let accountant = Arc::new(Budget::new(budget));
        let memory = Memory::from(Arc::clone(&accountant));
        let io0 = write_bytes();
        let t0 = Instant::now();
        let (sink, added) =
            add_in_adjacency(FileSink::open(work)?, &memory, &scratch, threads, &Silent)?;
        let seconds = t0.elapsed().as_secs_f64();
        let written = write_bytes() - io0;
        drop(sink);
        std::fs::remove_dir_all(&scratch_dir)?;
        if !added {
            return Err("the base file already holds an in-adjacency".into());
        }
        let file = Container::open(MmapSource::open(work)?)?;
        let len = |name: &str| file.get(name).map_or(0, |e| e.len);
        let index = file.get(OUT_INDEX).ok_or("no out-adjacency")?;
        let mut head = vec![0_u8; 32];
        file.source().read_at(index.offset, &mut head)?;
        let entries = u64::from_le_bytes(head[24..32].try_into()?);
        let bits = |bytes: u64| bytes as f64 * 8.0 / entries.max(1) as f64;
        let checked = match check {
            Some(dir) => {
                compare(work, dir)?;
                "true"
            }
            None => "null",
        };
        println!(
            "{{\"elapsed_seconds\":{seconds:.3},\"threads\":{threads},\"budget\":{budget},\
             \"peak_budget_bytes\":{},\"write_bytes\":{written},\"entries\":{entries},\
             \"in_data_bytes\":{},\"in_index_bytes\":{},\"in_bits_per_entry\":{:.2},\
             \"out_bits_per_entry\":{:.2},\"matches_reference\":{checked}}}",
            accountant.peak(),
            len(IN_DATA),
            len(IN_INDEX),
            bits(len(IN_DATA) + len(IN_INDEX)),
            bits(len(OUT_DATA) + len(OUT_INDEX)),
        );
        Ok(())
    }

    pub(crate) fn main() -> Result<(), BoxError> {
        let mut args = std::env::args().skip(1);
        let mode = args.next().ok_or("prepare or run")?;
        let (mut csr, mut out, mut base, mut work, mut budget, mut check) =
            (None, None, None, None, 24 << 30, None);
        while let Some(flag) = args.next() {
            let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--csr" => csr = Some(PathBuf::from(value)),
                "--out" => out = Some(PathBuf::from(value)),
                "--base" => base = Some(PathBuf::from(value)),
                "--work" => work = Some(PathBuf::from(value)),
                "--budget" => budget = parse_bytes(&value)?,
                "--check" => check = Some(PathBuf::from(value)),
                _ => return Err(format!("unknown flag {flag}").into()),
            }
        }
        match mode.as_str() {
            "prepare" => prepare(&csr.ok_or("--csr DIR")?, &out.ok_or("--out FILE")?),
            "run" => run(
                &base.ok_or("--base FILE")?,
                &work.ok_or("--work FILE")?,
                budget,
                check.as_deref(),
            ),
            _ => Err(format!("unknown mode {mode}").into()),
        }
    }
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), bench::BoxError> {
    bench::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("transpose reads /proc/self/io: Linux only");
}
