// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A build from a file: the job's scratch directory, the name of the graph made from an
//! input, a plain copy of a compressed input, and the build written so that no graph
//! file is ever half-written (`docs/ux-contract.md` § 5.2, C-1). The same functions as
//! `everygraph_ingest::native` of #81, the Python package.

#![allow(
    dead_code,
    reason = "the whole of `everygraph_ingest::native`; the command writes files only"
)]

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use everygraph_core::{ByteSource, Memory, MemorySource};
use everygraph_format::container::Provenance;
use everygraph_io::{FileSink, MmapSource};

use everygraph_ingest::build::{self, BuildError, BuildOptions, Checked, Phase, Progress, Report};
use everygraph_ingest::ids::Scratch;
use everygraph_ingest::text::{TextError, TextOptions, TextReader};

/// The scratch directory of one job, `.everygraph-ID` in the scratch root
/// (`docs/ux-contract.md` § 5.3), removed on drop. On Unix its files are unlinked as
/// soon as they are made, so a hard stop leaves at most the empty directory, which the
/// next run of the same job takes over.
#[derive(Debug)]
pub struct JobScratch {
    dir: PathBuf,
    next: AtomicU64,
}

impl JobScratch {
    /// Creates `.everygraph-JOB` in `root`, or takes over the one a stopped run left.
    ///
    /// # Errors
    ///
    /// The directory cannot be made.
    pub fn create(root: &Path, job: &str) -> io::Result<Self> {
        let dir = root.join(format!(".everygraph-{job}"));
        match std::fs::create_dir(&dir) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            other => other?,
        }
        Ok(Self {
            dir,
            next: AtomicU64::new(0),
        })
    }

    /// The directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for JobScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Scratch for JobScratch {
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
        // Elsewhere an open file cannot be unlinked; the directory's removal takes it.
        #[cfg(unix)]
        std::fs::remove_file(&path)?;
        Ok(file)
    }

    /// Maps the file: it is never written again (the map's invariant, `MmapSource`).
    fn seal(&self, file: File) -> io::Result<MmapSource> {
        MmapSource::from_file(&file)
    }
}

/// `NAME.eg` beside the input, `NAME` being its name without the extensions an edge
/// list may have (`transfers.csv.gz` → `transfers.eg`; `docs/api-sketch.md` § 2.7).
#[must_use]
pub fn default_output(input: &Path) -> PathBuf {
    const EXTENSIONS: [&str; 9] = [
        "gz", "zst", "txt", "csv", "tsv", "edges", "el", "edgelist", "ssv",
    ];
    let mut name = input
        .file_name()
        .map_or_else(|| "graph".to_owned(), |n| n.to_string_lossy().into_owned());
    while let Some((stem, ext)) = name.rsplit_once('.') {
        if stem.is_empty() || !EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            break;
        }
        name = stem.to_owned();
    }
    input.with_file_name(format!("{name}.eg"))
}

/// The first 16 hexadecimal digits of a job's fingerprint (`docs/ux-contract.md` R-4,
/// G-19): XXH3-128 of `parts` joined by NUL bytes. The parts are what names the work:
/// the input, the output and every option that changes the result, never the budget or
/// the threads.
#[must_use]
pub fn job_id(parts: &[&str]) -> String {
    let hash = xxhash_rust::xxh3::xxh3_128(parts.join("\0").as_bytes());
    format!("{:016x}", hash >> 64)
}

/// Why a build to a file failed.
#[derive(Debug)]
pub enum WriteError {
    /// The build failed; nothing was written under the output's name.
    Build(BuildError),
    /// A file could not be made, written or renamed.
    Io {
        /// The file.
        path: PathBuf,
        /// What the system said.
        error: io::Error,
    },
}

impl From<BuildError> for WriteError {
    fn from(e: BuildError) -> Self {
        Self::Build(e)
    }
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> WriteError + '_ {
    move |error| WriteError::Io {
        path: path.to_path_buf(),
        error,
    }
}

/// A plain copy of the compressed input of `reader`, in `scratch`, read through a map
/// and opened with `options`. The copy is unlinked at once and lives as long as the map.
///
/// # Errors
///
/// [`WriteError::Build`] when the input cannot be decompressed; [`WriteError::Io`] when
/// the copy cannot be written.
pub fn decompress<S: ByteSource + Sync>(
    reader: &TextReader<S>,
    scratch: &JobScratch,
    options: &TextOptions,
) -> Result<TextReader<MmapSource>, WriteError> {
    let path = scratch.dir().join("input.txt");
    let file = File::create(&path).map_err(io_error(&path))?;
    let mut writer = BufWriter::with_capacity(1 << 20, file);
    reader
        .decompress_into(&mut writer)
        .map_err(|e| WriteError::Build(BuildError::Text(e)))?;
    writer
        .into_inner()
        .map_err(|e| io_error(&path)(e.into_error()))?;
    let copy = MmapSource::open(&path).map_err(io_error(&path))?;
    let _ = std::fs::remove_file(&path);
    TextReader::open(copy, options).map_err(|e: TextError| WriteError::Build(e.into()))
}

/// Where [`write()`] puts the graph.
#[derive(Clone, Copy, Debug)]
pub enum Output<'a> {
    /// A file: written as `PATH.partial`, checked, then renamed to `PATH`.
    File(&'a Path),
    /// Bytes in memory.
    Memory,
}

/// What [`write()`] made.
#[derive(Debug)]
pub enum Written {
    /// The file, of this many bytes.
    File(u64),
    /// The bytes of the graph.
    Memory(Vec<u8>),
}

/// Removes `PATH.partial` unless the graph was renamed into place.
struct Partial(Option<PathBuf>);

impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Builds the graph of the plain-text `reader` ([`build::build`]), commits it with
/// `provenance`, reads it back and checks it ([`build::check`], the phase `checking`),
/// and puts it at `output`. A file is written as `PATH.partial` and renamed over
/// `PATH` only once checked; a failure at any point removes the partial file.
///
/// # Errors
///
/// [`WriteError::Build`] for a failure of the build or of the check;
/// [`WriteError::Io`] when the file cannot be made or renamed.
pub fn write<S: ByteSource + Sync>(
    reader: &TextReader<S>,
    options: &BuildOptions,
    memory: &Memory,
    scratch: &JobScratch,
    output: Output<'_>,
    provenance: Provenance,
    progress: &dyn Progress,
) -> Result<(Written, Report, Checked), WriteError> {
    let checked = |progress: &dyn Progress, source: &dyn Fn() -> Result<Checked, BuildError>| {
        progress.begin(Phase::Checking, None);
        let checked = source()?;
        let sections = u64::try_from(checked.sections).unwrap_or(u64::MAX);
        progress.end(Phase::Checking, sections);
        Ok::<_, WriteError>(checked)
    };
    match output {
        Output::Memory => {
            let sink = MemorySource::new(Vec::new());
            let unfinished = build::build(reader, options, memory, scratch, sink, progress)?;
            let (sink, report) = unfinished.commit(provenance)?;
            let bytes = sink.into_inner();
            let done = checked(progress, &|| {
                build::check(MemorySource::new(&bytes[..]), memory)
            })?;
            Ok((Written::Memory(bytes), report, done))
        }
        Output::File(path) => {
            let partial_path = PathBuf::from(format!("{}.partial", path.display()));
            let sink = FileSink::create(&partial_path).map_err(io_error(&partial_path))?;
            let mut partial = Partial(Some(partial_path.clone()));
            let unfinished = build::build(reader, options, memory, scratch, sink, progress)?;
            let (sink, report) = unfinished.commit(provenance)?;
            drop(sink.into_file());
            let written = MmapSource::open(&partial_path).map_err(io_error(&partial_path))?;
            let done = checked(progress, &|| build::check(&written, memory))?;
            drop(written);
            everygraph_io::persist(&partial_path, path).map_err(io_error(path))?;
            partial.0 = None;
            let bytes = std::fs::metadata(path).map_or(0, |m| m.len());
            Ok((Written::File(bytes), report, done))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use everygraph_core::MemorySource;

    use super::*;
    use everygraph_ingest::build::{GraphOptions, Silent};

    /// A directory of its own under the system's temporary directory, removed on drop.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let pid = std::process::id();
            let dir = std::env::temp_dir().join(format!("eg-native-{name}-{pid}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a temporary directory");
            Self(dir)
        }

        fn names(&self) -> Vec<std::ffi::OsString> {
            let entries = std::fs::read_dir(&self.0).expect("the directory");
            entries.map(|e| e.expect("an entry").file_name()).collect()
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_text(
        text: &str,
        dir: &Dir,
        output: Output<'_>,
    ) -> Result<(Written, Report, Checked), WriteError> {
        let source = MemorySource::new(text.as_bytes());
        let reader = TextReader::open(source, &TextOptions::default()).expect("an edge list");
        let options = BuildOptions::new(GraphOptions::default(), NonZeroUsize::MIN);
        let scratch = JobScratch::create(&dir.0, &job_id(&["test"])).expect("scratch");
        let memory = Memory::unlimited();
        let provenance = Provenance::default();
        write(
            &reader, &options, &memory, &scratch, output, provenance, &Silent,
        )
    }

    #[test]
    fn the_output_drops_the_extensions_of_an_edge_list() {
        let out = |p| default_output(Path::new(p));
        assert_eq!(out("a/transfers.csv.gz"), Path::new("a/transfers.eg"));
        assert_eq!(out("lj.txt"), Path::new("lj.eg"));
        assert_eq!(out("graph.v2.tsv"), Path::new("graph.v2.eg"));
        assert_eq!(out(".csv"), Path::new(".csv.eg"));
    }

    #[test]
    fn a_job_is_sixteen_hex_digits_of_its_parts() {
        let id = job_id(&["a", "b"]);
        assert_eq!(id.len(), 16);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(id, job_id(&["a", "b"]));
        assert_ne!(id, job_id(&["ab"]));
    }

    #[test]
    fn a_file_appears_whole_and_the_scratch_goes() {
        let dir = Dir::new("whole");
        let out = dir.0.join("g.eg");
        let (written, report, checked) =
            write_text("1 2\n2 3\n", &dir, Output::File(&out)).expect("a graph");
        let Written::File(bytes) = written else {
            panic!("a file")
        };
        assert_eq!(bytes, std::fs::metadata(&out).expect("the file").len());
        assert_eq!((report.nodes, report.edges, checked.nodes), (3, 2, 3));
        assert_eq!(dir.names(), ["g.eg"], "no partial file and no scratch");
    }

    #[test]
    fn a_failed_build_leaves_nothing() {
        let dir = Dir::new("failed");
        let out = dir.0.join("g.eg");
        let failed = write_text("1 2\n3\n", &dir, Output::File(&out));
        let refused = matches!(failed, Err(WriteError::Build(BuildError::Text(_))));
        assert!(refused, "{failed:?}");
        assert!(dir.names().is_empty(), "{:?}", dir.names());
    }

    #[test]
    fn a_graph_in_memory_is_checked_too() {
        let dir = Dir::new("memory");
        let (written, _, checked) = write_text("1 2\n", &dir, Output::Memory).expect("a graph");
        assert!(matches!(written, Written::Memory(b) if !b.is_empty()));
        assert_eq!(checked.nodes, 2);
    }
}
