// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A build from a file on a native build (feature `native`, which brings in
//! `everygraph-io`): the job's scratch directory, the name of the graph made from an
//! input, the identity of an input and whether a file already holds its graph, a plain
//! copy of a compressed input, and the build written so that no graph file is ever
//! half-written (`docs/ux-contract.md` § 5.2, C-1).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use everygraph_core::{ByteSource, Memory, MemorySource};
use everygraph_format::container::{Container, Input, Provenance};
use everygraph_io::{FileSink, FileSource, MmapSource};

use crate::build::{self, BuildError, BuildOptions, Checked, PROVENANCE, Phase, Progress, Report};
use crate::ids::Scratch;
use crate::text::{TextError, TextOptions, TextReader};

/// The scratch directory of one job, `.everygraph-ID` in the scratch root
/// (`docs/ux-contract.md` § 5.3), removed on drop. On Unix every file in it, the plain
/// copy of a compressed input included, is unlinked as soon as it is made, so a hard
/// stop leaves at most the empty directory, which the next run of the same job takes
/// over.
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

    /// A run stopped between making a file and unlinking it leaves the file behind under
    /// a name this run would use again; such a name is skipped, and the directory's
    /// removal takes the file.
    fn create(&self, purpose: &'static str) -> io::Result<File> {
        loop {
            let i = self.next.fetch_add(1, Ordering::Relaxed);
            let path = self
                .dir
                .join(format!("{i:08}-{}", purpose.replace(' ', "-")));
            let file = match File::options()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                other => other?,
            };
            // Elsewhere an open file cannot be unlinked; the directory's removal takes it.
            #[cfg(unix)]
            std::fs::remove_file(&path)?;
            return Ok(file);
        }
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

/// `path` made absolute, with `.` and `..` resolved and links followed: the file's own
/// name, so that two spellings of one file are one input or one output. A path that does
/// not exist yet is resolved through its directory.
#[must_use]
pub fn canonical(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    match (std::fs::canonicalize(dir), path.file_name()) {
        (Ok(dir), Some(name)) => dir.join(name),
        _ => std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()),
    }
}

/// Bytes hashed at each end of an input for its identity (R-6, a placeholder value).
const SAMPLE_BYTES: u64 = 1 << 20;

/// The identity of the input file at `path` (`docs/ux-contract.md` R-6): its canonical
/// name, its size, its modification time, and XXH3-128 of its first and last MiB.
/// Taken once, when the input is opened, and recorded in the build's provenance.
///
/// # Errors
///
/// The file cannot be opened or read.
pub fn identity(path: &Path) -> io::Result<Input> {
    let source = FileSource::open(path)?;
    let len = source.len();
    let head = len.min(SAMPLE_BYTES);
    let tail_start = len.saturating_sub(SAMPLE_BYTES).max(head);
    let mut hasher = xxhash_rust::xxh3::Xxh3::new();
    let mut buf = vec![0_u8; usize::try_from(SAMPLE_BYTES).unwrap_or(usize::MAX)];
    for range in [0..head, tail_start..len] {
        let n = usize::try_from(range.end - range.start).unwrap_or(0);
        source
            .read_at(range.start, &mut buf[..n])
            .map_err(io::Error::other)?;
        hasher.update(&buf[..n]);
    }
    let modified = std::fs::metadata(path)?
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_nanos()).ok());
    Ok(Input {
        name: canonical(path).display().to_string(),
        bytes: len,
        modified,
        algorithm: "xxh3-128-first-last-1MiB".into(),
        digest: hasher.digest128().to_le_bytes().to_vec(),
    })
}

/// What a file at the output's path holds, before a build writes there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// Nothing: there is no file.
    Nothing,
    /// The graph that the build would make: built from this input, unchanged, with these
    /// parameters. It is the cache (`docs/api-sketch.md` § 2.7).
    Same,
    /// Another graph.
    OtherGraph,
    /// Something Everygraph did not write, or a file that cannot be read.
    NotOurs,
}

/// What the file at `output` holds for a build of `input` with `parameters`
/// ([`build::parameters`]). Only a path that does not exist holds [`Held::Nothing`]: a
/// file that cannot be opened is [`Held::NotOurs`], and is never replaced unasked.
#[must_use]
pub fn held(output: &Path, input: &Input, parameters: &BTreeMap<String, String>) -> Held {
    let source = match FileSource::open(output) {
        Ok(source) => source,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Held::Nothing,
        Err(_) => return Held::NotOurs,
    };
    let Ok(file) = Container::open(source) else {
        return Held::NotOurs;
    };
    let Ok(stored) = file.read_provenance(PROVENANCE) else {
        return Held::OtherGraph;
    };
    let same_input = stored.inputs.first().is_some_and(|i| {
        (&i.name, i.bytes, i.modified, &i.algorithm, &i.digest)
            == (
                &input.name,
                input.bytes,
                input.modified,
                &input.algorithm,
                &input.digest,
            )
    });
    let same_parameters = parameters
        .iter()
        .all(|(k, v)| stored.parameters.get(k) == Some(v));
    if same_input && same_parameters {
        Held::Same
    } else {
        Held::OtherGraph
    }
}

/// Why a build to a file failed.
#[derive(Debug)]
pub enum WriteError {
    /// The build failed; nothing was written under the output's name.
    Build(BuildError),
    /// The caller withdrew the graph before it took its name: nothing was written.
    Withdrawn,
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
/// and opened with `options`. The copy is a scratch file, unlinked before its first
/// byte is written, so it lives exactly as long as the map.
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
    let dir = scratch.dir();
    let file = scratch.create("plain input").map_err(io_error(dir))?;
    let mut writer = BufWriter::with_capacity(1 << 20, &file);
    reader
        .decompress_into(&mut writer)
        .map_err(|e| WriteError::Build(BuildError::Text(e)))?;
    writer.flush().map_err(io_error(dir))?;
    drop(writer);
    let copy = scratch.seal(file).map_err(io_error(dir))?;
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
    /// The file, and its bytes.
    File(PathBuf, u64),
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
/// `PATH` only once checked and only if `keep()` says so, asked just before; a failure
/// or a withdrawal at any point removes the partial file, so a file already at `PATH`
/// is left as it was.
///
/// # Errors
///
/// [`WriteError::Build`] for a failure of the build or of the check;
/// [`WriteError::Withdrawn`] when `keep()` said no; [`WriteError::Io`] when the file
/// cannot be made or renamed.
#[expect(
    clippy::too_many_arguments,
    reason = "a build's inputs, its sink and its caller's gate"
)]
pub fn write<S: ByteSource + Sync>(
    reader: &TextReader<S>,
    options: &BuildOptions,
    memory: &Memory,
    scratch: &JobScratch,
    output: Output<'_>,
    provenance: Provenance,
    progress: &dyn Progress,
    keep: &dyn Fn() -> bool,
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
            if !keep() {
                return Err(WriteError::Withdrawn);
            }
            Ok((Written::Memory(bytes), report, done))
        }
        Output::File(path) => {
            let partial_path = PathBuf::from(format!("{}.partial", path.display()));
            let sink = FileSink::create(&partial_path).map_err(io_error(&partial_path))?;
            let mut partial = Partial(Some(partial_path.clone()));
            let unfinished = build::build(reader, options, memory, scratch, sink, progress)?;
            let (sink, report) = unfinished.commit(provenance)?;
            drop(sink);
            let written = MmapSource::open(&partial_path).map_err(io_error(&partial_path))?;
            let done = checked(progress, &|| build::check(&written, memory))?;
            drop(written);
            if !keep() {
                return Err(WriteError::Withdrawn);
            }
            everygraph_io::persist(&partial_path, path).map_err(io_error(path))?;
            partial.0 = None;
            let bytes = std::fs::metadata(path).map_or(0, |m| m.len());
            Ok((Written::File(path.to_path_buf(), bytes), report, done))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use everygraph_core::MemorySource;

    use super::*;
    use crate::build::{GraphOptions, Silent};

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
            &reader,
            &options,
            &memory,
            &scratch,
            output,
            provenance,
            &Silent,
            &|| true,
        )
    }

    /// A killed run can leave a scratch file under the first name the next run makes
    /// (`a_killed_ingest_leaves_no_damaged_graph` once failed so).
    #[test]
    fn a_scratch_file_left_by_a_stopped_run_is_skipped() {
        let dir = Dir::new("stale-scratch");
        let job = job_id(&["stale"]);
        let left = dir.0.join(format!(".everygraph-{job}"));
        std::fs::create_dir(&left).expect("the stopped run's directory");
        std::fs::write(left.join("00000000-ids"), b"stale").expect("its file");
        let scratch = JobScratch::create(&dir.0, &job).expect("taken over");
        scratch
            .create("ids")
            .expect("a new file beside the stale one");
        drop(scratch);
        assert!(dir.names().is_empty(), "{:?}", dir.names());
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
        let Written::File(_, bytes) = written else {
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

    #[test]
    fn a_withdrawn_graph_leaves_the_file_in_the_way_as_it_was() {
        let dir = Dir::new("withdrawn");
        let out = dir.0.join("g.eg");
        std::fs::write(&out, b"mine").expect("a file");
        let source = MemorySource::new(&b"1 2\n"[..]);
        let reader = TextReader::open(source, &TextOptions::default()).expect("an edge list");
        let options = BuildOptions::new(GraphOptions::default(), NonZeroUsize::MIN);
        let scratch = JobScratch::create(&dir.0, "w").expect("scratch");
        let memory = Memory::unlimited();
        let output = Output::File(&out);
        let provenance = Provenance::default();
        let withdrawn = write(
            &reader,
            &options,
            &memory,
            &scratch,
            output,
            provenance,
            &Silent,
            &|| false,
        );
        assert!(
            matches!(withdrawn, Err(WriteError::Withdrawn)),
            "{withdrawn:?}"
        );
        drop(scratch);
        assert_eq!(std::fs::read(&out).expect("the file"), b"mine");
        assert_eq!(dir.names(), ["g.eg"]);
    }

    #[test]
    fn only_a_missing_path_holds_nothing() {
        let dir = Dir::new("held");
        let input = dir.0.join("e.txt");
        std::fs::write(&input, b"1 2\n").expect("an input");
        let identity = identity(&input).expect("its identity");
        let layout = TextReader::open(MemorySource::new(&b"1 2\n"[..]), &TextOptions::default())
            .expect("an edge list")
            .layout()
            .clone();
        let parameters = build::parameters(&GraphOptions::default(), &layout);
        assert_eq!(
            held(&dir.0.join("none.eg"), &identity, &parameters),
            Held::Nothing
        );
        assert_eq!(held(&input, &identity, &parameters), Held::NotOurs);
        assert_eq!(
            held(&dir.0, &identity, &parameters),
            Held::NotOurs,
            "a directory"
        );
        let out = dir.0.join("g.eg");
        let source = MemorySource::new(&b"1 2\n"[..]);
        let reader = TextReader::open(source, &TextOptions::default()).expect("an edge list");
        let options = BuildOptions::new(GraphOptions::default(), NonZeroUsize::MIN);
        let scratch = JobScratch::create(&dir.0, "h").expect("scratch");
        let provenance = Provenance {
            inputs: vec![identity.clone()],
            ..Provenance::default()
        };
        let memory = Memory::unlimited();
        write(
            &reader,
            &options,
            &memory,
            &scratch,
            Output::File(&out),
            provenance,
            &Silent,
            &|| true,
        )
        .expect("a graph");
        assert_eq!(held(&out, &identity, &parameters), Held::Same);
        let directed = GraphOptions {
            directed: true,
            ..GraphOptions::default()
        };
        let other = build::parameters(&directed, &layout);
        assert_eq!(held(&out, &identity, &other), Held::OtherGraph);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = dir.0.join("locked.eg");
            std::fs::write(&locked, b"mine").expect("a file");
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
                .expect("mode");
            let readable = std::fs::File::open(&locked).is_ok();
            if !readable {
                assert_eq!(held(&locked, &identity, &parameters), Held::NotOurs);
            }
        }
    }

    #[test]
    fn the_identity_sees_a_change_of_the_same_size_and_time() {
        let dir = Dir::new("identity");
        let input = dir.0.join("e.txt");
        std::fs::write(&input, b"1,2\n2,3\n").expect("an input");
        let first = identity(&input).expect("its identity");
        let time = std::fs::metadata(&input)
            .and_then(|m| m.modified())
            .expect("a time");
        std::fs::write(&input, b"1,2\n2,4\n").expect("the same size");
        let file = std::fs::File::options()
            .write(true)
            .open(&input)
            .expect("the file");
        file.set_modified(time).expect("the old time");
        let second = identity(&input).expect("its identity");
        assert_eq!(
            (first.bytes, first.modified),
            (second.bytes, second.modified)
        );
        assert_ne!(first.digest, second.digest);
        assert_eq!(
            identity(&dir.0.join(".").join("e.txt"))
                .expect("again")
                .name,
            first.name
        );
    }

    #[test]
    fn the_plain_copy_of_a_compressed_input_has_no_name() {
        use std::io::Write as _;
        let dir = Dir::new("plain");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(b"1 2\n2 3\n").expect("gzip");
        let packed = gz.finish().expect("gzip");
        let reader = TextReader::open(MemorySource::new(&packed[..]), &TextOptions::default())
            .expect("a compressed edge list");
        let scratch = JobScratch::create(&dir.0, "p").expect("scratch");
        let plain = decompress(&reader, &scratch, &TextOptions::default()).expect("a copy");
        assert_eq!(plain.source().len(), 8);
        let names = std::fs::read_dir(scratch.dir())
            .expect("the scratch")
            .count();
        if cfg!(unix) {
            assert_eq!(names, 0, "the copy is unlinked");
        }
    }
}
