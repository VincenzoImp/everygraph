// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test helpers: a temporary directory and a small HTTP/1.1 file server on 127.0.0.1
//! that can honour or ignore `Range`, and cut responses short to simulate a dropped
//! connection.

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

/// A directory under the system's temporary directory, removed on drop.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(crate) fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "everygraph-bench-test-{}-{tag}-{n}",
            std::process::id()
        ));
        let _ignored = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ignored = std::fs::remove_dir_all(&self.path);
    }
}

/// How the server behaves.
#[derive(Debug, Clone, Default)]
pub(crate) struct ServerConfig {
    /// Ignore `Range` and always send the whole body with 200.
    pub(crate) ignore_ranges: bool,
    /// For the first this-many body responses (the probe is not counted), send only
    /// `cut_after` bytes of the body and then close the connection.
    pub(crate) cut_responses: usize,
    /// See `cut_responses`.
    pub(crate) cut_after: usize,
    /// Report this total size in `Content-Range` instead of the real one.
    pub(crate) lie_about_total: Option<u64>,
}

/// A request the server saw: the `Range` header, if any.
pub(crate) type Seen = Option<String>;

pub(crate) struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Seen>>>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Serves `body` at `/file`; every other path is 404.
    pub(crate) fn start(body: Vec<u8>, config: ServerConfig) -> Self {
        // Never 0.0.0.0: this machine has a public address.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1");
        let port = listener.local_addr().expect("addr").port();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let body = Arc::new(body);
        let cuts_left = Arc::new(Mutex::new(config.cut_responses));
        let thread = {
            let stop = Arc::clone(&stop);
            let seen = Arc::clone(&seen);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let body = Arc::clone(&body);
                    let seen = Arc::clone(&seen);
                    let cuts_left = Arc::clone(&cuts_left);
                    let config = config.clone();
                    std::thread::spawn(move || {
                        let _ignored = serve(stream, &body, &config, &seen, &cuts_left);
                    });
                }
            })
        };
        Self {
            port,
            stop,
            seen,
            thread: Some(thread),
        }
    }

    pub(crate) fn url(&self) -> String {
        format!("http://127.0.0.1:{}/file", self.port)
    }

    pub(crate) fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The `Range` headers of the requests served so far, in arrival order.
    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ignored = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(t) = self.thread.take() {
            let _ignored = t.join();
        }
    }
}

fn serve(
    stream: TcpStream,
    body: &[u8],
    config: &ServerConfig,
    seen: &Mutex<Vec<Seen>>,
    cuts_left: &Mutex<usize>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let path = request_line.split(' ').nth(1).unwrap_or("").to_string();
    let mut range = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_string());
            }
        }
    }
    let mut out = stream;
    if path != "/file" {
        out.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    }
    seen.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(range.clone());
    let total = body.len();
    let parsed = range.as_deref().and_then(|r| {
        let (a, b) = r.strip_prefix("bytes=")?.split_once('-')?;
        let a: usize = a.parse().ok()?;
        let b: usize = if b.is_empty() {
            total - 1
        } else {
            b.parse().ok()?
        };
        (a <= b && b < total).then_some((a, b))
    });
    let (status, slice, extra) = match parsed {
        Some((a, b)) if !config.ignore_ranges => {
            let reported = config
                .lie_about_total
                .unwrap_or(u64::try_from(total).unwrap_or(u64::MAX));
            (
                "206 Partial Content",
                body.get(a..=b).unwrap_or_default(),
                format!("Content-Range: bytes {a}-{b}/{reported}\r\n"),
            )
        }
        _ => ("200 OK", body, String::new()),
    };
    let is_probe = range.as_deref() == Some("bytes=0-0");
    write!(
        out,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n{extra}Connection: close\r\n\r\n",
        slice.len()
    )?;
    let cut = !is_probe && {
        let mut left = cuts_left.lock().unwrap_or_else(PoisonError::into_inner);
        let some = *left > 0;
        *left = left.saturating_sub(1);
        some
    };
    if cut {
        out.write_all(
            slice
                .get(..config.cut_after.min(slice.len()))
                .unwrap_or_default(),
        )?;
        out.flush()?;
        out.shutdown(Shutdown::Both)?;
        return Ok(());
    }
    out.write_all(slice)?;
    out.flush()
}

/// Deterministic pseudo-random bytes.
pub(crate) fn bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()[0]
        })
        .collect()
}
