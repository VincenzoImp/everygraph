// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Sequential scan of one file, for the acceptance of issue #29 (`bench/specs/io.toml`).
//!
//! ```text
//! scan <raw|direct|pread|mmap> <file> [--threads N] [--segment-mib M]
//! ```
//!
//! Every mode splits the file into segments of M MiB (default 8), hands them to N
//! threads (default 32) through a shared counter, and sums the bytes as 64-bit words so
//! that every byte is touched. `direct`, `pread` and `mmap` read through the crate's
//! backends and `SegmentReader`; `raw` is the reference that does not use the crate:
//! plain `pread` on a file opened with `O_DIRECT`, into one aligned buffer per thread.
//! Prints one JSON line.

#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::fs::File;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use everygraph_core::{AccessHint, AlignedBuf, ByteSource, SegmentReader, Segments};
use everygraph_io::{FileSource, MmapSource};

fn checksum(bytes: &[u8]) -> u64 {
    let words = bytes.chunks_exact(8);
    let tail = words
        .remainder()
        .iter()
        .fold(0_u64, |s, &b| s.wrapping_add(u64::from(b)));
    words.fold(tail, |s, w| {
        s.wrapping_add(u64::from_le_bytes(w.try_into().unwrap_or_default()))
    })
}

/// Runs `per_segment` on every segment index from `threads` workers; returns the sum.
fn scan<W: FnMut(u64) -> Result<u64, String> + Send>(
    segments: &Segments,
    threads: usize,
    make_worker: impl Fn() -> W + Sync,
) -> Result<u64, String> {
    let next = AtomicU64::new(0);
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut work = make_worker();
                    let mut sum = 0_u64;
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= segments.count_all() {
                            return Ok::<u64, String>(sum);
                        }
                        sum = sum.wrapping_add(work(i)?);
                    }
                })
            })
            .collect();
        workers.into_iter().try_fold(0_u64, |total, w| {
            Ok(total.wrapping_add(w.join().map_err(|_| "a worker panicked".to_string())??))
        })
    })
}

fn through_source(
    source: &(dyn ByteSource + Sync),
    segments: &Segments,
    threads: usize,
) -> Result<u64, String> {
    scan(segments, threads, || {
        let mut reader = SegmentReader::new();
        move |i| {
            let range = segments.get(i).ok_or("no such segment")?;
            Ok(checksum(
                reader.read(source, range).map_err(|e| e.to_string())?,
            ))
        }
    })
}

fn raw(path: &str, segments: &Segments, threads: usize) -> Result<(u64, bool), String> {
    let mut options = File::options();
    options.read(true);
    #[cfg(target_os = "linux")]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_DIRECT);
    let file = &options.open(path).map_err(|e| e.to_string())?;
    let size = usize::try_from(segments.size()).map_err(|e| e.to_string())?;
    let sum = scan(segments, threads, || {
        let mut buf = AlignedBuf::try_zeroed(size);
        move |i| {
            let range = segments.get(i).ok_or("no such segment")?;
            let buf = buf.as_mut().map_err(|e| e.to_string())?.as_mut_slice();
            let want = usize::try_from(range.end - range.start).map_err(|e| e.to_string())?;
            // O_DIRECT reads whole blocks; the last segment is read rounded up and the
            // read stops at the end of the file.
            let rounded = want
                .next_multiple_of(everygraph_core::SEGMENT_ALIGN)
                .min(buf.len());
            let mut got = 0;
            while got < want {
                let (piece, at) = (&mut buf[got..rounded], range.start + got as u64);
                #[cfg(unix)]
                let n = std::os::unix::fs::FileExt::read_at(file, piece, at);
                #[cfg(windows)]
                let n = std::os::windows::fs::FileExt::seek_read(file, piece, at);
                #[cfg(not(any(unix, windows)))]
                let n: std::io::Result<usize> = Err(std::io::ErrorKind::Unsupported.into());
                match n.map_err(|e| e.to_string())? {
                    0 => return Err("unexpected end of file".into()),
                    n => got += n,
                }
            }
            Ok(checksum(&buf[..want]))
        }
    })?;
    Ok((sum, cfg!(target_os = "linux")))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str, default: u64| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map_or(Ok(default), |v| {
                v.parse::<u64>().map_err(|e| format!("{name}: {e}"))
            })
    };
    let run = || -> Result<String, String> {
        let [mode, path] = [args.first(), args.get(1)].map(|a| a.map(String::as_str));
        let (Some(mode), Some(path)) = (mode, path) else {
            return Err(
                "usage: scan <raw|direct|pread|mmap> <file> [--threads N] [--segment-mib M]".into(),
            );
        };
        let threads = usize::try_from(flag("--threads", 32)?).map_err(|e| e.to_string())?;
        let segment = flag("--segment-mib", 8)? << 20;
        let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        let segments = Segments::new(len, segment);
        let start = Instant::now();
        let (sum, direct) = match mode {
            "raw" => raw(path, &segments, threads)?,
            "direct" => {
                let source = FileSource::open_direct(path).map_err(|e| e.to_string())?;
                (
                    through_source(&source, &segments, threads)?,
                    source.is_direct(),
                )
            }
            "pread" => {
                let source = FileSource::open(path).map_err(|e| e.to_string())?;
                source.advise(0..len, AccessHint::Sequential);
                (through_source(&source, &segments, threads)?, false)
            }
            "mmap" => {
                let source = MmapSource::open(path).map_err(|e| e.to_string())?;
                source.advise(0..len, AccessHint::Sequential);
                (through_source(&source, &segments, threads)?, false)
            }
            other => return Err(format!("unknown mode {other}")),
        };
        let seconds = start.elapsed().as_secs_f64();
        #[allow(clippy::cast_precision_loss)]
        let gb_per_s = len as f64 / seconds / 1e9;
        Ok(format!(
            "{{\"mode\":\"{mode}\",\"direct\":{direct},\"threads\":{threads},\"segment_bytes\":{},\
             \"bytes\":{len},\"seconds\":{seconds:.6},\"gb_per_s\":{gb_per_s:.4},\"checksum\":{sum}}}",
            segments.size()
        ))
    };
    match run() {
        Ok(line) => println!("{line}"),
        Err(e) => {
            eprintln!("scan: {e}");
            std::process::exit(1);
        }
    }
}
