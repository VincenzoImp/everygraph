// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Parallel ranged HTTP download with resume.
//!
//! The file is split into chunks of at most [`MAX_CHUNK`] bytes; `connections` workers
//! take chunks from a shared queue and write each at its offset in a preallocated
//! `.part` file. Many chunks rather than one range per connection, so that one slow
//! connection does not hold up the end. A text `.state` file records how many bytes of
//! each chunk are written; it is saved every second, so an interrupted download (Ctrl-C,
//! network loss, a killed process) resumes from where each chunk stopped. A failed
//! request is retried from its own progress, with backoff.
//!
//! The state file is a promise about bytes handed to the operating system, not about
//! bytes on the platter: after a machine crash some recorded bytes may be lost. The
//! checksum check after the download catches that, and the download starts over.
//!
//! A server that ignores `Range` gets one plain stream, without resume.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ureq::Agent;
use ureq::http::Response;
use ureq::tls::{Certificate, RootCerts, TlsConfig, TlsProvider};

use super::error::{Context, Error, Result};
use super::store::{human_bytes, write_atomically};

/// The largest chunk one request asks for.
pub const MAX_CHUNK: u64 = 64 << 20;
/// The smallest chunk, so that small files are not cut into many tiny requests.
pub const MIN_CHUNK: u64 = 1 << 20;

/// How a download behaves on the network.
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// Parallel connections per file. SNAP serves about 0.7 MB/s per connection; the
    /// lab measured about 17 MB/s with 8.
    pub connections: usize,
    /// Retries of one chunk after a failed request, before the download gives up.
    pub retries: u32,
    /// Wait before the first retry; doubled for each further one, at most 30 s.
    pub backoff: Duration,
    /// A read that stalls longer than this fails (and is retried).
    pub read_timeout: Duration,
    /// How often progress is written to the state file.
    pub save_every: Duration,
    /// How often a progress line is logged; `None` for never.
    pub log_every: Option<Duration>,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            connections: 8,
            retries: 5,
            backoff: Duration::from_secs(1),
            read_timeout: Duration::from_secs(60),
            save_every: Duration::from_secs(1),
            log_every: Some(Duration::from_secs(10)),
        }
    }
}

/// Builds the HTTP client: rustls with ring, the operating system's root certificates,
/// no compression (bytes arrive as stored, so ranges and checksums mean what they say).
///
/// # Errors
/// Never in practice; the signature leaves room for failing to set up TLS.
pub fn agent(options: &DownloadOptions) -> Result<Agent> {
    // An empty store only makes https fail later, with a TLS error that names the host;
    // plain http (the tests' local server) does not need it.
    let native = rustls_native_certs::load_native_certs();
    let roots: Vec<Certificate<'static>> = native
        .certs
        .iter()
        .map(|c| Certificate::from_der(c.as_ref()).to_owned())
        .collect();
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(RootCerts::new_with_certs(&roots))
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    let config = Agent::config_builder()
        .tls_config(tls)
        .user_agent(concat!(
            "everygraph-bench/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/VincenzoImp/everygraph)"
        ))
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_per_read(Some(options.read_timeout))
        .build();
    Ok(Agent::new_with_config(config))
}

/// One chunk of the file: `start..end`, of which `done` bytes from `start` are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chunk {
    start: u64,
    end: u64,
    done: u64,
}

/// The progress of a download, as saved in the `.state` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct State {
    size: u64,
    chunks: Vec<Chunk>,
}

const STATE_MAGIC: &str = "everygraph-partial 1";

impl State {
    fn plan(size: u64, connections: usize) -> Self {
        let parts = u64::try_from(connections.max(1)).unwrap_or(1);
        let chunk = size.div_ceil(parts).clamp(MIN_CHUNK, MAX_CHUNK);
        let mut chunks = Vec::new();
        let mut start = 0;
        while start < size {
            let end = (start + chunk).min(size);
            chunks.push(Chunk {
                start,
                end,
                done: 0,
            });
            start = end;
        }
        Self { size, chunks }
    }

    /// Bytes written so far.
    pub(crate) fn done(&self) -> u64 {
        self.chunks.iter().map(|c| c.done).sum()
    }

    fn to_text(&self) -> String {
        let mut s = format!("{STATE_MAGIC}\nsize {}\n", self.size);
        for c in &self.chunks {
            s.push_str(&format!("{} {} {}\n", c.start, c.end, c.done));
        }
        s
    }

    /// Reads a state file; anything inconsistent is an error, and the caller starts over.
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).context(|| format!("in {}", path.display()))
    }

    fn parse(text: &str) -> Result<Self> {
        let bad = || Error::new("malformed download state");
        let mut lines = text.lines();
        if lines.next() != Some(STATE_MAGIC) {
            return Err(bad());
        }
        let size: u64 = lines
            .next()
            .and_then(|l| l.strip_prefix("size "))
            .and_then(|v| v.parse().ok())
            .ok_or_else(bad)?;
        let mut chunks = Vec::new();
        let mut expected_start = 0;
        for line in lines {
            let mut it = line.split(' ').map(str::parse::<u64>);
            let (Some(Ok(start)), Some(Ok(end)), Some(Ok(done)), None) =
                (it.next(), it.next(), it.next(), it.next())
            else {
                return Err(bad());
            };
            if start != expected_start || end <= start || done > end - start {
                return Err(bad());
            }
            expected_start = end;
            chunks.push(Chunk { start, end, done });
        }
        if expected_start != size {
            return Err(bad());
        }
        Ok(Self { size, chunks })
    }
}

/// What the first request told us about the server.
enum Probe {
    /// The server honours `Range`; the file has this many bytes.
    Ranges { total: u64 },
    /// The server sent the whole file; here is the response to stream.
    Whole(Response<ureq::Body>),
}

/// Downloads `url` into `part`, resuming from `state_path` when it describes the same
/// file. On success `part` holds `size` bytes and the state file is removed; the caller
/// still has to verify the checksum.
///
/// # Errors
/// If the server is unreachable, the size differs from `expected_size`, or a chunk
/// still fails after the retries. The state file is kept, so a later call resumes.
pub fn download(
    agent: &Agent,
    url: &str,
    expected_size: Option<u64>,
    part: &Path,
    state_path: &Path,
    options: &DownloadOptions,
    log: &mut dyn Write,
) -> Result<u64> {
    match probe(agent, url)? {
        Probe::Ranges { total } => {
            if let Some(expected) = expected_size {
                if expected != total {
                    return Err(Error::new(format!(
                        "{url} has {total} bytes, the registry says {expected}: the source changed"
                    )));
                }
            }
            ranged(agent, url, total, part, state_path, options, log)?;
            Ok(total)
        }
        Probe::Whole(response) => {
            let _ignored = fs::remove_file(state_path);
            whole(response, url, expected_size, part)
        }
    }
}

fn probe(agent: &Agent, url: &str) -> Result<Probe> {
    let response = agent
        .get(url)
        .header("Range", "bytes=0-0")
        .call()
        .context(|| format!("GET {url}"))?;
    match response.status().as_u16() {
        206 => {
            let total = content_range(&response)
                .and_then(|(_, _, total)| total)
                .ok_or_else(|| Error::new(format!("{url}: 206 without a usable Content-Range")))?;
            Ok(Probe::Ranges { total })
        }
        200 => Ok(Probe::Whole(response)),
        other => Err(Error::new(format!("GET {url}: unexpected status {other}"))),
    }
}

/// Parses `Content-Range: bytes a-b/total` (total may be `*`).
fn content_range<B>(response: &Response<B>) -> Option<(u64, u64, Option<u64>)> {
    let value = response.headers().get("content-range")?.to_str().ok()?;
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (a, b) = range.split_once('-')?;
    let total = if total == "*" {
        None
    } else {
        Some(total.parse().ok()?)
    };
    Some((a.parse().ok()?, b.parse().ok()?, total))
}

fn whole(
    mut response: Response<ureq::Body>,
    url: &str,
    expected_size: Option<u64>,
    part: &Path,
) -> Result<u64> {
    let mut out = File::create(part).context(|| format!("creating {}", part.display()))?;
    let mut reader = response.body_mut().as_reader();
    let written = std::io::copy(&mut reader, &mut out).context(|| format!("downloading {url}"))?;
    out.sync_all()
        .context(|| format!("syncing {}", part.display()))?;
    if let Some(expected) = expected_size {
        if written != expected {
            return Err(Error::new(format!(
                "{url} sent {written} bytes, the registry says {expected}"
            )));
        }
    }
    Ok(written)
}

/// Shared between the workers of one download.
struct Shared {
    /// Bytes written per chunk.
    done: Mutex<Vec<u64>>,
    /// Index into `pending` of the next chunk to take.
    next: AtomicUsize,
    /// Set when a worker gave up: the others stop at their next read.
    abort: AtomicBool,
    /// The first error, reported by the download.
    error: Mutex<Option<Error>>,
}

fn ranged(
    agent: &Agent,
    url: &str,
    total: u64,
    part: &Path,
    state_path: &Path,
    options: &DownloadOptions,
    log: &mut dyn Write,
) -> Result<()> {
    let resumable = State::load(state_path)
        .ok()
        .filter(|s| s.size == total)
        .filter(|_| fs::metadata(part).is_ok_and(|m| m.len() == total));
    let mut state = if let Some(state) = resumable {
        let _ignored = writeln!(
            log,
            "  resuming: {} of {} already written",
            human_bytes(state.done()),
            human_bytes(total)
        );
        state
    } else {
        let f = File::create(part).context(|| format!("creating {}", part.display()))?;
        f.set_len(total)
            .context(|| format!("allocating {}", part.display()))?;
        let state = State::plan(total, options.connections);
        write_atomically(state_path, state.to_text().as_bytes())?;
        state
    };

    let pending: Vec<usize> = (0..state.chunks.len())
        .filter(|&i| {
            state
                .chunks
                .get(i)
                .is_some_and(|c| c.done < c.end - c.start)
        })
        .collect();
    let shared = Shared {
        done: Mutex::new(state.chunks.iter().map(|c| c.done).collect()),
        next: AtomicUsize::new(0),
        abort: AtomicBool::new(false),
        error: Mutex::new(None),
    };
    let chunks = state.chunks.clone();
    let workers = options.connections.max(1).min(pending.len());
    let started = Instant::now();
    let done_at_start = state.done();

    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| worker(agent, url, total, part, &chunks, &pending, &shared, options))
            })
            .collect();
        let mut last_save = Instant::now();
        let mut last_log = Instant::now();
        while !handles
            .iter()
            .all(std::thread::ScopedJoinHandle::is_finished)
        {
            std::thread::sleep(Duration::from_millis(50));
            if last_save.elapsed() >= options.save_every {
                save(&shared, &mut state, state_path);
                last_save = Instant::now();
            }
            if let Some(every) = options.log_every {
                if last_log.elapsed() >= every {
                    let done = progress(&shared);
                    let millis = started.elapsed().as_millis().max(1);
                    let rate = u128::from(done.saturating_sub(done_at_start)) * 1000 / millis;
                    let percent = u128::from(done) * 100 / u128::from(total.max(1));
                    let _ignored = writeln!(
                        log,
                        "  {} of {} ({percent}%), {}/s",
                        human_bytes(done),
                        human_bytes(total),
                        human_bytes(u64::try_from(rate).unwrap_or(u64::MAX)),
                    );
                    last_log = Instant::now();
                }
            }
        }
        for h in handles {
            if h.join().is_err() {
                record(&shared, Error::new("a download thread panicked"));
            }
        }
    });
    save(&shared, &mut state, state_path);
    if let Some(e) = shared
        .error
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    {
        return Err(e);
    }
    if state.done() != total {
        return Err(Error::new(format!(
            "{url}: {} of {total} bytes written",
            state.done()
        )));
    }
    File::open(part)
        .and_then(|f| f.sync_all())
        .context(|| format!("syncing {}", part.display()))?;
    fs::remove_file(state_path).context(|| format!("removing {}", state_path.display()))
}

fn progress(shared: &Shared) -> u64 {
    shared
        .done
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .sum()
}

fn save(shared: &Shared, state: &mut State, state_path: &Path) {
    {
        let done = shared.done.lock().unwrap_or_else(PoisonError::into_inner);
        for (c, d) in state.chunks.iter_mut().zip(done.iter()) {
            c.done = *d;
        }
    }
    // A failed save loses at most the progress since the last one; not worth failing for.
    let _ignored = write_atomically(state_path, state.to_text().as_bytes());
}

fn record(shared: &Shared, error: Error) {
    shared.abort.store(true, Ordering::SeqCst);
    let mut slot = shared.error.lock().unwrap_or_else(PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(error);
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "a thread body; bundling the arguments would only move them"
)]
fn worker(
    agent: &Agent,
    url: &str,
    total: u64,
    part: &Path,
    chunks: &[Chunk],
    pending: &[usize],
    shared: &Shared,
    options: &DownloadOptions,
) {
    let mut file = match OpenOptions::new().write(true).open(part) {
        Ok(f) => f,
        Err(e) => {
            return record(
                shared,
                Error::new(format!("opening {}: {e}", part.display())),
            );
        }
    };
    loop {
        if shared.abort.load(Ordering::SeqCst) {
            return;
        }
        let slot = shared.next.fetch_add(1, Ordering::SeqCst);
        let Some(&index) = pending.get(slot) else {
            return;
        };
        let Some(&chunk) = chunks.get(index) else {
            return;
        };
        let mut failures = 0;
        loop {
            match fetch_chunk(agent, url, total, &mut file, chunk, index, shared) {
                Ok(()) => break,
                Err(_) if shared.abort.load(Ordering::SeqCst) => return,
                Err(e) => {
                    failures += 1;
                    if failures > options.retries {
                        return record(
                            shared,
                            Error::new(format!(
                                "bytes {}..{} of {url} failed {failures} times; last error: {e}",
                                chunk.start, chunk.end
                            )),
                        );
                    }
                    let wait = options
                        .backoff
                        .saturating_mul(1 << (failures - 1).min(5))
                        .min(Duration::from_secs(30));
                    std::thread::sleep(wait);
                }
            }
        }
    }
}

/// Fetches what is missing of one chunk with one request.
fn fetch_chunk(
    agent: &Agent,
    url: &str,
    total: u64,
    file: &mut File,
    chunk: Chunk,
    index: usize,
    shared: &Shared,
) -> Result<()> {
    let done = shared
        .done
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(index)
        .copied()
        .unwrap_or(0);
    let start = chunk.start + done;
    if start >= chunk.end {
        return Ok(());
    }
    let last = chunk.end - 1;
    let mut response = agent
        .get(url)
        .header("Range", format!("bytes={start}-{last}"))
        .call()
        .context(|| format!("GET {url} bytes {start}-{last}"))?;
    if response.status().as_u16() != 206 {
        return Err(Error::new(format!(
            "asked for bytes {start}-{last}, got status {}",
            response.status()
        )));
    }
    if content_range(&response) != Some((start, last, Some(total))) {
        return Err(Error::new(format!(
            "asked for bytes {start}-{last}/{total}, got Content-Range {:?}",
            response.headers().get("content-range")
        )));
    }
    file.seek(SeekFrom::Start(start))
        .context(|| "seeking in the partial file".to_string())?;
    let mut reader = response.body_mut().as_reader();
    let mut buf = vec![0_u8; 256 << 10];
    let mut offset = start;
    while offset < chunk.end {
        if shared.abort.load(Ordering::SeqCst) {
            return Err(Error::new("aborted"));
        }
        let want = usize::try_from(chunk.end - offset)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let slice = buf.get_mut(..want).unwrap_or_default();
        let n = reader
            .read(slice)
            .context(|| format!("reading {url} at byte {offset}"))?;
        if n == 0 {
            return Err(Error::new(format!(
                "{url}: connection closed at byte {offset}, expected up to {}",
                chunk.end
            )));
        }
        file.write_all(slice.get(..n).unwrap_or_default())
            .context(|| "writing the partial file".to_string())?;
        let n = u64::try_from(n).unwrap_or(0);
        offset += n;
        if let Some(d) = shared
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(index)
        {
            *d += n;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_tile_the_file_and_respect_the_bounds() {
        for (size, conns) in [(0, 8), (1, 8), (5 << 20, 8), (1 << 30, 8), (10 << 30, 4)] {
            let s = State::plan(size, conns);
            assert_eq!(s.chunks.iter().map(|c| c.end - c.start).sum::<u64>(), size);
            let mut at = 0;
            for c in &s.chunks {
                assert_eq!(c.start, at);
                assert!(c.end - c.start <= MAX_CHUNK);
                at = c.end;
            }
        }
        assert_eq!(State::plan(5 << 20, 8).chunks.len(), 5, "1 MiB minimum");
        assert_eq!(State::plan(10 << 30, 8).chunks.len(), 160, "64 MiB maximum");
    }

    #[test]
    fn state_round_trips_and_rejects_inconsistent_text() {
        let mut s = State::plan(3 << 20, 3);
        if let Some(c) = s.chunks.get_mut(1) {
            c.done = 1234;
        }
        let text = s.to_text();
        assert_eq!(State::parse(&text).expect("parses"), s);
        assert_eq!(s.done(), 1234);
        for broken in [
            text.replace(STATE_MAGIC, "something else"),
            text.replace("size 3145728", "size 3145729"),
            text.replace(" 1234\n", " 99999999\n"),
            text.replace("1048576 2097152", "1048577 2097152"),
            format!("{text}garbage\n"),
        ] {
            assert!(State::parse(&broken).is_err(), "{broken}");
        }
    }
}
