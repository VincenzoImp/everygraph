// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exploratory, interleaved A/B of the graph build (issue #35) at several budgets: one
//! process, the budgets taking turns, each repetition a full build to a `.eg` file
//! with `fsync`. Prints one JSON line per run: seconds (build and commit apart), the
//! bytes this process wrote (`/proc/self/io` `write_bytes`), what spilled, the output.
//!
//! ```text
//! build_ab --input FILE --out DIR --budgets 8G,256M [--reps N] [--directed yes]
//! ```
//!
//! Threads: `RAYON_NUM_THREADS`. Not a measurement for the record: the harness runs
//! those (`bench/specs/build.toml`).

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a benchmark binary reports on its streams"
)]

#[cfg(target_os = "linux")]
mod bench {
    use std::fs::File;
    use std::io;
    use std::num::NonZeroUsize;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    use everygraph_core::{Budget, Memory};
    use everygraph_format::container::Provenance;
    use everygraph_ingest::build::{self, BuildOptions, GraphOptions, Silent};
    use everygraph_ingest::ids::Scratch;
    use everygraph_ingest::text::{TextOptions, TextReader};
    use everygraph_io::{FileSink, FileSource, MmapSource};

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
            "K" => 10,
            "M" => 20,
            "G" => 30,
            _ => return Err(format!("bad size {s}").into()),
        };
        Ok(num.parse::<u64>()? << shift)
    }

    fn run_one(input: &Path, out: &Path, budget: u64, directed: bool) -> Result<String, BoxError> {
        let scratch_dir = out.with_extension("scratch");
        std::fs::create_dir_all(&scratch_dir)?;
        let scratch = DirScratch {
            dir: scratch_dir.clone(),
            next: AtomicU64::new(0),
        };
        let threads = std::env::var("RAYON_NUM_THREADS")
            .ok()
            .and_then(|t| t.parse().ok())
            .and_then(NonZeroUsize::new)
            .unwrap_or(NonZeroUsize::new(32).expect("non-zero"));
        let accountant = Arc::new(Budget::new(budget));
        let memory = Memory::from(Arc::clone(&accountant));
        let io0 = write_bytes();
        let t0 = Instant::now();
        let reader = TextReader::open(FileSource::open(input)?, &TextOptions::default())?;
        let graph = GraphOptions {
            directed,
            ..GraphOptions::default()
        };
        let options = BuildOptions::new(graph, threads);
        let sink = FileSink::create(out)?;
        let unfinished = build::build(&reader, &options, &memory, &scratch, sink, &Silent)?;
        let built = t0.elapsed().as_secs_f64();
        let t1 = Instant::now();
        let (sink, report) = unfinished.commit(Provenance {
            engine: "everygraph build_ab".into(),
            ..Provenance::default()
        })?;
        let committed = t1.elapsed().as_secs_f64();
        let written = write_bytes() - io0;
        let output = sink.into_file().metadata()?.len();
        std::fs::remove_dir_all(&scratch_dir)?;
        Ok(format!(
            "{{\"budget\":{budget},\"regime\":\"{:?}\",\"build_seconds\":{built:.3},\
             \"commit_seconds\":{committed:.3},\"write_bytes\":{written},\
             \"spilled_bytes\":{},\"output_bytes\":{output},\"edges\":{},\"nodes\":{},\
             \"buckets\":{},\"peak_budget_bytes\":{},\"bytes_per_edge\":{:.2}}}",
            report.regime,
            report.spilled_bytes,
            report.edges,
            report.nodes,
            report.buckets,
            accountant.peak(),
            written as f64 / report.edges.max(1) as f64,
        ))
    }

    pub(crate) fn main() -> Result<(), BoxError> {
        let (mut input, mut out, mut budgets, mut reps, mut directed) =
            (None, None, Vec::new(), 3, false);
        let mut it = std::env::args().skip(1);
        while let Some(flag) = it.next() {
            let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--input" => input = Some(PathBuf::from(value)),
                "--out" => out = Some(PathBuf::from(value)),
                "--budgets" => {
                    budgets = value
                        .split(',')
                        .map(parse_bytes)
                        .collect::<Result<_, _>>()?;
                }
                "--reps" => reps = value.parse()?,
                "--directed" => directed = value == "yes",
                _ => return Err(format!("unknown flag {flag}").into()),
            }
        }
        let input = input.ok_or("--input FILE")?;
        let out = out.ok_or("--out DIR")?;
        std::fs::create_dir_all(&out)?;
        for rep in 0..reps {
            for &budget in &budgets {
                let file = out.join(format!("ab-{budget}.eg"));
                let line = run_one(&input, &file, budget, directed)?;
                println!("{{\"rep\":{rep},{}", &line[1..]);
                std::fs::remove_file(&file)?;
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), bench::BoxError> {
    bench::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("build_ab reads /proc/self/io: Linux only");
}
