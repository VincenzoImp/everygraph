// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Benchmark of the identifier dictionary (issue #34), run by the harness
//! (`bench/specs/id-dictionary.toml`). Prints one JSON line.
//!
//! ```text
//! id_dictionary --keys eth|ethbin|user|int|sparse|file:PATH [--n N] [--stream M]
//!     [--budget BYTES] [--scratch memory|dir:PATH] [--layout split|slot]
//!     [--order natural|first|sorted] [--lookups Q] [--map M] [--pregenerate yes]
//!     [--cold Q] [--build-only yes]
//! ```
//!
//! `--keys`: synthetic Ethereum-like addresses as 42-character text (`eth`) or 20
//! bytes (`ethbin`), URL-like user names (`user`), integers `0..n` (`int`) or spread
//! over 64 bits (`sparse`), or the first two
//! tab-separated columns of a text file (`file:`). `--n` distinct keys, read `--stream`
//! times (default: each once, in a scrambled order; more: drawn with repetition).
//! `--map M` maps M keys drawn from the set after the build (the second pass of an
//! ingest), in parallel, and reports the rate with and without generating the keys.
//! Threads: `RAYON_NUM_THREADS`.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a benchmark binary reports on its streams"
)]

#[cfg(target_os = "linux")]
mod bench {
    use std::fs::File;
    use std::io::{self, BufRead, Write};
    use std::num::NonZeroUsize;
    use std::os::unix::fs::FileExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    use everygraph_core::{
        ByteSource, InsufficientBudget, MemoryAccountant, MemoryKind, MemorySource, NodeId,
        ReadError, par,
    };
    use everygraph_ingest::ids::{
        BuildOptions, Collector, ColumnType, Dictionary, DictionaryError, IdOrder, Key, KeyKind,
        Layout, MemoryScratch, SAMPLE_SIZE, Scratch,
    };

    pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync>;

    struct Args {
        keys: String,
        n: u64,
        stream: u64,
        budget: u64,
        scratch: String,
        layout: Layout,
        order: IdOrder,
        lookups: u64,
        map: u64,
        pregenerate: bool,
        pregenerated: bool,
        cold: u64,
        build_only: bool,
    }

    fn parse_args() -> Result<Args, BoxError> {
        let mut a = Args {
            keys: "eth".into(),
            n: 1_000_000,
            stream: 0,
            budget: 4 << 30,
            scratch: "memory".into(),
            layout: Layout::Split,
            order: IdOrder::Natural,
            lookups: 1_000_000,
            map: 0,
            pregenerate: false,
            pregenerated: false,
            cold: 0,
            build_only: false,
        };
        let mut it = std::env::args().skip(1);
        while let Some(flag) = it.next() {
            let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--keys" => a.keys = value,
                "--n" => a.n = value.parse()?,
                "--stream" => a.stream = value.parse()?,
                "--budget" => a.budget = parse_bytes(&value)?,
                "--scratch" => a.scratch = value,
                "--layout" => {
                    a.layout = match value.as_str() {
                        "split" => Layout::Split,
                        "slot" => Layout::SlotOrdered,
                        _ => return Err("--layout split|slot".into()),
                    }
                }
                "--order" => {
                    a.order = match value.as_str() {
                        "natural" => IdOrder::Natural,
                        "first" => IdOrder::FirstAppearance,
                        "sorted" => IdOrder::SortedKey,
                        _ => return Err("--order natural|first|sorted".into()),
                    }
                }
                "--lookups" => a.lookups = value.parse()?,
                "--map" => a.map = value.parse()?,
                "--pregenerate" => a.pregenerate = value == "yes",
                "--cold" => a.cold = value.parse()?,
                "--build-only" => a.build_only = value == "yes",
                _ => return Err(format!("unknown flag {flag}").into()),
            }
        }
        Ok(a)
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

    fn splitmix(mut z: u64) -> u64 {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    const HEX: &[u8; 16] = b"0123456789abcdef";
    const SITES: [&str; 6] = [
        "https://twitter.com/",
        "https://www.instagram.com/",
        "https://github.com/",
        "https://www.facebook.com/profile/",
        "https://t.me/",
        "https://mastodon.social/@",
    ];

    /// The keys: synthetic (key `i` computed on demand) or read from a file.
    enum KeySet {
        Synthetic(String),
        Loaded(Vec<Vec<u8>>),
        /// Synthetic keys of one width generated before the clock starts, so that their
        /// generation is not timed (`--pregenerate`).
        Flat {
            kind: String,
            width: usize,
            data: Vec<u8>,
        },
    }

    impl KeySet {
        fn column(&self) -> ColumnType {
            match self {
                Self::Synthetic(k) | Self::Flat { kind: k, .. } if k == "ethbin" => {
                    ColumnType::FixedBinary { width: 20 }
                }
                Self::Synthetic(k) if k == "int" || k == "sparse" => ColumnType::UInt { bits: 64 },
                _ => ColumnType::Utf8,
            }
        }

        /// Writes key `i` into `out` (or returns it as an integer).
        fn key<'b>(&'b self, i: u64, out: &'b mut Vec<u8>) -> Key<'b> {
            out.clear();
            match self {
                Self::Loaded(keys) => Key::Bytes(&keys[usize::try_from(i).expect("in memory")]),
                Self::Flat { width, data, .. } => {
                    let at = usize::try_from(i).expect("in memory") * width;
                    Key::Bytes(&data[at..at + width])
                }
                Self::Synthetic(kind) => match kind.as_str() {
                    "int" => Key::UInt(i),
                    "sparse" => Key::UInt(splitmix(i)),
                    "eth" | "ethbin" => {
                        let mut b = [0_u8; 24];
                        for (j, chunk) in b.chunks_mut(8).enumerate() {
                            chunk.copy_from_slice(
                                &splitmix(i ^ (u64::try_from(j).expect("small") + 1) << 56)
                                    .to_le_bytes(),
                            );
                        }
                        if kind == "ethbin" {
                            out.extend_from_slice(&b[..20]);
                        } else {
                            out.extend_from_slice(b"0x");
                            for &x in &b[..20] {
                                out.push(HEX[usize::from(x >> 4)]);
                                out.push(HEX[usize::from(x & 15)]);
                            }
                        }
                        Key::Bytes(out)
                    }
                    _ => {
                        let h = splitmix(i);
                        out.extend_from_slice(
                            SITES[usize::try_from(h % 6).expect("small")].as_bytes(),
                        );
                        out.extend_from_slice(b"user");
                        out.extend_from_slice(i.to_string().as_bytes());
                        Key::Bytes(out)
                    }
                },
            }
        }
    }

    fn load_file(path: &str) -> Result<Vec<Vec<u8>>, BoxError> {
        let mut seen = std::collections::HashSet::new();
        let mut keys = Vec::new();
        for line in io::BufReader::new(File::open(path)?).split(b'\n') {
            let line = line?;
            for field in line.split(|&c| c == b'\t').take(2) {
                if !field.is_empty() && seen.insert(field.to_vec()) {
                    keys.push(field.to_vec());
                }
            }
        }
        Ok(keys)
    }

    /// The test accountant of the benchmark: a fixed budget with its peak.
    struct Budget {
        total: u64,
        used: AtomicU64,
        peak: AtomicU64,
    }

    impl MemoryAccountant for Budget {
        fn try_reserve(
            &self,
            purpose: &'static str,
            kind: MemoryKind,
            bytes: u64,
        ) -> Result<(), InsufficientBudget> {
            if kind != MemoryKind::Anonymous {
                return Ok(());
            }
            let mut used = self.used.load(Ordering::Relaxed);
            loop {
                let Some(after) = used.checked_add(bytes).filter(|a| *a <= self.total) else {
                    return Err(InsufficientBudget::new(purpose, bytes, self.total - used));
                };
                match self.used.compare_exchange_weak(
                    used,
                    after,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        self.peak.fetch_max(after, Ordering::Relaxed);
                        return Ok(());
                    }
                    Err(now) => used = now,
                }
            }
        }

        fn release(&self, _: &'static str, kind: MemoryKind, bytes: u64) {
            if kind == MemoryKind::Anonymous {
                self.used.fetch_sub(bytes, Ordering::Relaxed);
            }
        }

        fn available(&self) -> u64 {
            self.total - self.used.load(Ordering::Relaxed)
        }
    }

    /// Scratch files in a directory, deleted when their source is dropped. Writers have
    /// no buffer of their own: the dictionary buffers what it writes in 64 KiB that it
    /// reserves, and a buffer here would be heap the budget does not count (a 1 MiB one
    /// per writer, with about 190 writers open, put the heap up to 114 MiB above the
    /// ledger at 50 M keys).
    struct DirScratch {
        dir: PathBuf,
        next: AtomicU64,
    }

    struct DirWriter {
        file: File,
        path: PathBuf,
    }

    impl Write for DirWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.file.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.file.flush()
        }
    }

    struct DirSource {
        file: File,
        len: u64,
        path: PathBuf,
    }

    impl ByteSource for DirSource {
        fn len(&self) -> u64 {
            self.len
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
            self.file
                .read_exact_at(buf, offset)
                .map_err(|e| ReadError::Backend(Box::new(e)))
        }
    }

    /// Sources that can be pushed out of the page cache, for cold look-ups.
    trait Evict {
        fn sync(&self) {}
        fn evict(&self) {}
    }

    impl Evict for MemorySource<Vec<u8>> {}

    impl Evict for DirSource {
        fn sync(&self) {
            let _ = self.file.sync_all();
        }
        fn evict(&self) {
            let _ = rustix::fs::fadvise(&self.file, 0, None, rustix::fs::Advice::DontNeed);
        }
    }

    impl Drop for DirSource {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    impl Scratch for DirScratch {
        type Source = DirSource;
        type Writer = DirWriter;

        fn create(&self, purpose: &'static str) -> io::Result<DirWriter> {
            let i = self.next.fetch_add(1, Ordering::Relaxed);
            let path = self
                .dir
                .join(format!("{i:08}-{}", purpose.replace(' ', "-")));
            Ok(DirWriter {
                file: File::create(&path)?,
                path,
            })
        }

        fn seal(&self, writer: DirWriter) -> io::Result<DirSource> {
            let DirWriter { file, path } = writer;
            drop(file);
            let file = File::open(&path)?;
            let len = file.metadata()?.len();
            Ok(DirSource { file, len, path })
        }
    }

    /// Counts the reads made through it while `on` (a shared counter would slow the
    /// parallel passes down, so it is switched off for them).
    struct Counted<'s, S> {
        inner: &'s S,
        reads: &'s AtomicU64,
        on: &'s std::sync::atomic::AtomicBool,
    }

    impl<S: ByteSource> ByteSource for Counted<'_, S> {
        fn len(&self) -> u64 {
            self.inner.len()
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
            if self.on.load(Ordering::Relaxed) {
                self.reads.fetch_add(1, Ordering::Relaxed);
            }
            self.inner.read_at(offset, buf)
        }
    }

    const CHUNK: u64 = 1 << 16;

    fn occurrence(args: &Args, j: u64) -> u64 {
        if args.pregenerated {
            // Pregenerated keys are stored in stream order.
            j
        } else if args.stream == args.n {
            // Each key once, scrambled: j -> (j * P + 12345) mod n is a bijection of 0..n
            // when the prime P does not divide n.
            const P: u128 = 1_000_000_007;
            let n = u128::from(args.n);
            u64::try_from((u128::from(j) * P + 12_345) % n).expect("below n")
        } else {
            splitmix(j ^ 0xABCD) % args.n
        }
    }

    fn percentile(sorted: &[f64], p: f64) -> f64 {
        if sorted.is_empty() {
            return f64::NAN;
        }
        // Nearest rank, without a float-to-integer cast.
        let last = sorted.len() - 1;
        let at = (0..=last)
            .find(|&i| i as f64 >= last as f64 * p - 0.5)
            .unwrap_or(last);
        sorted[at]
    }

    pub(crate) fn main() -> Result<(), BoxError> {
        // Before any thread exists: glibc must not keep freed blocks that the budget no
        // longer counts (a stopgap until issue m2-19-allocator-policy).
        everygraph_io::limit_allocator_retention();
        let mut args = parse_args()?;
        let keys = match args.keys.strip_prefix("file:") {
            Some(path) => {
                let loaded = load_file(path)?;
                args.n = u64::try_from(loaded.len())?;
                KeySet::Loaded(loaded)
            }
            None if args.pregenerate && args.stream != 0 && args.stream != args.n => {
                return Err("--pregenerate reads each key once: leave --stream out".into());
            }
            None if args.pregenerate => {
                args.stream = args.n;
                let width = match args.keys.as_str() {
                    "eth" => 42,
                    "ethbin" => 20,
                    _ => return Err("--pregenerate is for eth and ethbin keys".into()),
                };
                let synthetic = KeySet::Synthetic(args.keys.clone());
                let mut data = vec![0_u8; usize::try_from(args.n)? * width];
                par::for_each_chunk_mut(
                    &mut data,
                    NonZeroUsize::new(width << 12).expect("non-zero"),
                    |start, chunk| {
                        let mut b = Vec::with_capacity(64);
                        for (j, out) in chunk.chunks_exact_mut(width).enumerate() {
                            let i =
                                occurrence(&args, u64::try_from(start / width + j).expect("index"));
                            if let Key::Bytes(k) = synthetic.key(i, &mut b) {
                                out.copy_from_slice(k);
                            }
                        }
                    },
                );
                args.pregenerated = true;
                KeySet::Flat {
                    kind: args.keys.clone(),
                    width,
                    data,
                }
            }
            None => KeySet::Synthetic(args.keys.clone()),
        };
        if args.stream == 0 {
            args.stream = args.n;
        }
        let column = keys.column();
        let budget = Budget {
            total: args.budget,
            used: AtomicU64::new(0),
            peak: AtomicU64::new(0),
        };
        let dir_scratch;
        let result = if let Some(dir) = args.scratch.strip_prefix("dir:") {
            std::fs::create_dir_all(dir)?;
            dir_scratch = DirScratch {
                dir: dir.into(),
                next: AtomicU64::new(0),
            };
            run(&args, &keys, column, &budget, &dir_scratch)?
        } else {
            run(&args, &keys, column, &budget, &MemoryScratch)?
        };
        println!("{result}");
        Ok(())
    }

    fn run<Sc: Scratch>(
        args: &Args,
        keys: &KeySet,
        column: ColumnType,
        budget: &Budget,
        scratch: &Sc,
    ) -> Result<String, BoxError>
    where
        Sc::Source: Evict,
    {
        // Kind from the first keys of the stream, as `build_dictionary` does.
        let mut sample_bufs: Vec<Vec<u8>> = Vec::new();
        let mut sample_ints = Vec::new();
        for j in 0..args.stream.min(u64::try_from(SAMPLE_SIZE)?) {
            let mut b = Vec::new();
            match keys.key(occurrence(args, j), &mut b) {
                Key::UInt(v) => sample_ints.push(Key::UInt(v)),
                Key::Int(v) => sample_ints.push(Key::Int(v)),
                Key::Bytes(k) => sample_bufs.push(k.to_vec()),
            }
        }
        let kind = KeyKind::detect(
            column,
            sample_ints
                .into_iter()
                .chain(sample_bufs.iter().map(|b| Key::Bytes(b))),
        );
        let mut options = BuildOptions::default()
            .with_layout(args.layout)
            .with_order(args.order);

        let t0 = Instant::now();
        let chunks = usize::try_from(args.stream.div_ceil(CHUNK))?;
        // As `build_dictionary` does: a pass that meets an integer outside 0..2^32 (or a
        // bitmap the budget refuses) starts again in the hashed form.
        let distinct = loop {
            let collector = Collector::new(column, kind, &options, budget, scratch)?;
            let results: Vec<Result<(), DictionaryError>> =
                par::map_ranges(chunks, NonZeroUsize::MIN, |r| {
                    let mut w = collector.worker()?;
                    let start = u64::try_from(r.start).expect("chunk") * CHUNK;
                    let mut buf = Vec::with_capacity(64);
                    for j in start..(start + CHUNK).min(args.stream) {
                        let key = keys.key(occurrence(args, j), &mut buf);
                        w.add(j, key)?;
                    }
                    Ok(())
                });
            match results.into_iter().collect::<Result<(), _>>() {
                Err(DictionaryError::NotDense | DictionaryError::Budget(_)) if options.dense => {
                    options.dense = false;
                }
                other => {
                    other?;
                    break collector.finish()?;
                }
            }
        };
        let invalid = distinct.invalid().count;
        let distinct_s = t0.elapsed().as_secs_f64();
        let t1 = Instant::now();
        let dict = distinct.build()?;
        let phases_s = t1.elapsed().as_secs_f64();
        let build_s = t0.elapsed().as_secs_f64();
        let peak_reserved = budget.peak.load(Ordering::Relaxed);
        let n = u64::from(dict.len());
        let section_bytes: Vec<(String, u64)> = dict
            .sections()
            .map(|(name, s)| (name.to_string(), s.len()))
            .collect();
        let total_bytes: u64 = section_bytes.iter().map(|(_, b)| b).sum();
        eprintln!(
            "built {n} keys ({:?}) in {build_s:.2} s: distinct pass {distinct_s:.2} s, phases {phases_s:.2} s",
            dict.kind()
        );

        if args.build_only {
            // The build alone, for memory measurements: nothing is loaded afterwards.
            return Ok(format!(
                "{{\"keys\":\"{}\",\"kind\":\"{:?}\",\"n\":{},\"budget\":{},\"build_s\":{:.4},\"peak_reserved_bytes\":{},\"invalid\":{},\"bytes_per_key\":{:.3}}}",
                args.keys,
                kind,
                n,
                args.budget,
                build_s,
                peak_reserved,
                invalid,
                total_bytes as f64 / n.max(1) as f64
            ));
        }
        let cold = cold_lookups(args, keys, &dict)?;

        // Look-ups through in-memory copies of the sections when they fit the budget's
        // order of magnitude; otherwise through the scratch.
        let in_memory = total_bytes <= args.budget.max(1 << 33);
        let lookups = if in_memory {
            let sections: std::collections::BTreeMap<String, Vec<u8>> = dict
                .sections()
                .map(|(name, s)| {
                    let mut b = vec![0; usize::try_from(s.len()).expect("in memory")];
                    s.read_at(0, &mut b).map(|()| (name.to_string(), b))
                })
                .collect::<Result<_, _>>()?;
            drop(dict);
            let reads = AtomicU64::new(0);
            let on = std::sync::atomic::AtomicBool::new(false);
            let mem: std::collections::BTreeMap<&str, MemorySource<&[u8]>> = sections
                .iter()
                .map(|(k, v)| (k.as_str(), MemorySource::new(v.as_slice())))
                .collect();
            let d = Dictionary::open(budget, |name| {
                mem.get(name).map(|s| Counted {
                    inner: s,
                    reads: &reads,
                    on: &on,
                })
            })?;
            let prefix = prefix_search(args, keys, &d)?;
            format!("{},{prefix}", measure(args, keys, &d, &reads, &on)?)
        } else {
            let reads = AtomicU64::new(0);
            let on = std::sync::atomic::AtomicBool::new(false);
            measure(args, keys, &dict, &reads, &on)?
        };
        let sections_json: Vec<String> = section_bytes
            .iter()
            .map(|(k, v)| format!("\"{k}\":{v}"))
            .collect();
        let resident_bits = 8.0
            * (section_bytes
                .iter()
                .filter(|(k, _)| matches!(k.as_str(), "ids.parts" | "ids.pilots" | "ids.remap"))
                .map(|(_, b)| *b)
                .sum::<u64>() as f64)
            / n.max(1) as f64;
        Ok(format!(
            concat!(
                "{{\"keys\":\"{}\",\"kind\":\"{:?}\",\"n\":{},\"stream\":{},\"threads\":\"{}\",\"budget\":{},\"scratch\":\"{}\",",
                "\"layout\":\"{:?}\",\"order\":\"{:?}\",\"build_s\":{:.4},\"distinct_s\":{:.4},\"phases_s\":{:.4},",
                "\"keys_per_s\":{:.0},\"peak_reserved_bytes\":{},\"hash_function_bits_per_key\":{:.4},",
                "\"bytes_per_key\":{:.3},\"invalid\":{},\"sections\":{{{}}},{},{}}}"
            ),
            args.keys,
            kind,
            n,
            args.stream,
            std::env::var("RAYON_NUM_THREADS").unwrap_or_default(),
            args.budget,
            args.scratch,
            args.layout,
            args.order,
            build_s,
            distinct_s,
            phases_s,
            args.stream as f64 / build_s,
            peak_reserved,
            resident_bits,
            total_bytes as f64 / n.max(1) as f64,
            invalid,
            sections_json.join(","),
            lookups,
            cold,
        ))
    }

    /// Forward and reverse look-ups with every section evicted from the page cache before
    /// each one (the hash function stays in memory, as it does in use). Only with
    /// `--scratch dir:` and `--cold Q`.
    fn cold_lookups<S: ByteSource + Evict>(
        args: &Args,
        keys: &KeySet,
        d: &Dictionary<'_, S>,
    ) -> Result<String, BoxError> {
        if args.cold == 0 || !args.scratch.starts_with("dir:") {
            return Ok("\"cold\":null".into());
        }
        for (_, s) in d.sections() {
            s.sync();
        }
        let evict = || {
            for (_, s) in d.sections() {
                s.evict();
            }
        };
        let mut buf = Vec::new();
        let mut fwd = Vec::new();
        let mut rev = Vec::new();
        let mut out = Vec::new();
        for j in 0..args.cold {
            // Different keys for the two directions: the drive keeps its own cache.
            let key = keys.key(splitmix(j ^ 0xC01D) % args.n, &mut buf);
            evict();
            let t = Instant::now();
            let id = d.lookup(key)?.ok_or("a key of the set was not found")?;
            fwd.push(t.elapsed().as_nanos() as f64 / 1000.0);
            let other = NodeId::new(u32::try_from(splitmix(j ^ 0xBEEF) % u64::from(d.len()))?);
            evict();
            let t = Instant::now();
            d.normalized_key(other, &mut out)?;
            rev.push(t.elapsed().as_nanos() as f64 / 1000.0);
            std::hint::black_box(id);
        }
        fwd.sort_by(f64::total_cmp);
        rev.sort_by(f64::total_cmp);
        Ok(format!(
            "\"cold\":{{\"n\":{},\"fwd_p50_us\":{:.1},\"fwd_p95_us\":{:.1},\"rev_p50_us\":{:.1},\"rev_p95_us\":{:.1}}}",
            args.cold,
            percentile(&fwd, 0.5),
            percentile(&fwd, 0.95),
            percentile(&rev, 0.5),
            percentile(&rev, 0.95),
        ))
    }

    /// The prefix index: time to build it (first call) and warm prefix queries (100
    /// results at most) on prefixes of random keys less their last 8 bytes.
    fn prefix_search<S: ByteSource>(
        args: &Args,
        keys: &KeySet,
        d: &Dictionary<'_, S>,
    ) -> Result<String, BoxError> {
        if !matches!(d.kind(), KeyKind::Bytes | KeyKind::Fixed { .. }) {
            return Ok("\"prefix\":null".into());
        }
        let mut buf = Vec::new();
        let prefix_of = |j: u64, buf: &mut Vec<u8>| -> Vec<u8> {
            match keys.key(splitmix(j ^ 0xF00D) % args.n, buf) {
                Key::Bytes(k) => k[..k.len().saturating_sub(8).max(1)].to_vec(),
                _ => Vec::new(),
            }
        };
        let t = Instant::now();
        let first = match d.prefix_search(&prefix_of(0, &mut buf), 100) {
            Ok(first) => first,
            // The index does not fit what is left of the budget.
            Err(DictionaryError::Budget(_)) => {
                return Ok("\"prefix\":\"refused by the budget\"".into());
            }
            Err(e) => return Err(e.into()),
        };
        let build_s = t.elapsed().as_secs_f64();
        std::hint::black_box(first);
        let mut lat = Vec::new();
        let mut hits = 0;
        for j in 1..=1000 {
            let p = prefix_of(j, &mut buf);
            let t = Instant::now();
            hits += d.prefix_search(&p, 100)?.len();
            lat.push(t.elapsed().as_nanos() as f64 / 1000.0);
        }
        lat.sort_by(f64::total_cmp);
        Ok(format!(
            "\"prefix\":{{\"index_build_s\":{build_s:.3},\"p50_us\":{:.2},\"p95_us\":{:.2},\"avg_hits\":{:.1}}}",
            percentile(&lat, 0.5),
            percentile(&lat, 0.95),
            f64::from(u32::try_from(hits)?) / 1000.0
        ))
    }

    fn measure<S: ByteSource + Sync>(
        args: &Args,
        keys: &KeySet,
        d: &Dictionary<'_, S>,
        reads: &AtomicU64,
        on: &std::sync::atomic::AtomicBool,
    ) -> Result<String, BoxError> {
        let n = args.n;
        let q = args.lookups.min(1 << 24);
        let mut buf = Vec::new();
        // Warm-up pass, then single-thread latencies.
        for j in 0..q {
            let key = keys.key(splitmix(j) % n, &mut buf);
            d.lookup(key)?.ok_or("a key of the set was not found")?;
        }
        let mut lat = Vec::with_capacity(usize::try_from(q.min(200_000))?);
        for j in 0..q.min(200_000) {
            let key = keys.key(splitmix(j) % n, &mut buf);
            let t = Instant::now();
            let id = d.lookup(key)?;
            lat.push(t.elapsed().as_nanos() as f64 / 1000.0);
            std::hint::black_box(id);
        }
        lat.sort_by(f64::total_cmp);
        let mut rev = Vec::with_capacity(lat.len());
        let mut out = Vec::new();
        for j in 0..q.min(200_000) {
            let id = NodeId::new(u32::try_from(splitmix(j ^ 99) % u64::from(d.len()))?);
            let t = Instant::now();
            d.normalized_key(id, &mut out)?;
            rev.push(t.elapsed().as_nanos() as f64 / 1000.0);
        }
        rev.sort_by(f64::total_cmp);
        // Reads per forward look-up and per member mapping.
        reads.store(0, Ordering::Relaxed);
        on.store(true, Ordering::Relaxed);
        for j in 0..1000 {
            d.lookup(keys.key(splitmix(j) % n, &mut buf))?;
        }
        let reads_lookup = reads.swap(0, Ordering::Relaxed) as f64 / 1000.0;
        for j in 0..1000 {
            d.id_of_member(keys.key(splitmix(j) % n, &mut buf))?;
        }
        let reads_member = reads.swap(0, Ordering::Relaxed) as f64 / 1000.0;
        on.store(false, Ordering::Relaxed);
        // Parallel mapping of `--map` keys of the set, taken in turn (key j % n, so that
        // pregenerated keys are read sequentially, as a second pass reads its input):
        // reading the keys alone, then with verified look-ups, then member mapping.
        let mut mapping = String::from("\"map\":null");
        if args.map > 0 {
            let m = args.map;
            let chunks = usize::try_from(m.div_ceil(1 << 16))?;
            let pass = |mode: u8| -> Result<f64, BoxError> {
                let t = Instant::now();
                let sums: Vec<Result<u64, String>> =
                    par::map_ranges(chunks, NonZeroUsize::MIN, |r| {
                        let mut b = Vec::with_capacity(64);
                        let start = u64::try_from(r.start).expect("chunk") << 16;
                        let mut s = 0_u64;
                        for j in start..(start + (1 << 16)).min(m) {
                            let key = keys.key(j % n, &mut b);
                            s += match mode {
                                0 => u64::from(matches!(key, Key::Bytes(k) if k.len() > 1)),
                                1 => u64::from(
                                    d.lookup(key)
                                        .map_err(|e| e.to_string())?
                                        .ok_or("missing")?
                                        .get(),
                                ),
                                _ => u64::from(
                                    d.id_of_member(key)
                                        .map_err(|e| e.to_string())?
                                        .ok_or("missing")?
                                        .get(),
                                ),
                            };
                        }
                        Ok(s)
                    });
                let total: u64 = sums
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()?
                    .iter()
                    .sum();
                std::hint::black_box(total);
                Ok(t.elapsed().as_secs_f64())
            };
            let gen_s = pass(0)?;
            let verified_s = pass(1)?;
            let member_s = pass(2)?;
            mapping = format!(
                "\"map\":{{\"m\":{m},\"generate_s\":{gen_s:.4},\"verified_s\":{verified_s:.4},\"member_s\":{member_s:.4},\"verified_per_s\":{:.0},\"member_per_s\":{:.0}}}",
                m as f64 / verified_s,
                m as f64 / member_s
            );
        }
        Ok(format!(
            "\"fwd_warm_p50_us\":{:.4},\"fwd_warm_p95_us\":{:.4},\"fwd_warm_p99_us\":{:.4},\"rev_warm_p50_us\":{:.4},\"reads_per_lookup\":{reads_lookup:.3},\"reads_per_member_mapping\":{reads_member:.3},{mapping}",
            percentile(&lat, 0.5),
            percentile(&lat, 0.95),
            percentile(&lat, 0.99),
            percentile(&rev, 0.5),
        ))
    }
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), bench::BoxError> {
    bench::main()
}

/// The benchmark reads files with positional reads and evicts them with
/// `posix_fadvise`: Linux only.
#[cfg(not(target_os = "linux"))]
fn main() {}
