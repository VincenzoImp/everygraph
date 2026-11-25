// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Parses a text edge list with the text reader and prints one JSON line: what was
//! read, the best and median wall time of `--repeat` passes, and the parse rate.
//! Run by `bench/specs/text-reader.toml`; threads come from `RAYON_NUM_THREADS`.
//!
//! ```sh
//! cargo run --release -p everygraph-ingest --example text_parse -- FILE [--repeat N] [--digest]
//! ```
//!
//! `--digest` adds an order-sensitive hash of every (source, target) pair, to check that
//! plain, gzip and zstd copies of a file give the same rows; it is off for timings.
//! The file is read through a small positional-read adapter until `everygraph-io`
//! (#29) provides the file backend.

#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::fs::File;
use std::time::Instant;

use everygraph_core::{ByteSource, ReadError};
use everygraph_ingest::text::{Batch, Ids, TextOptions, TextReader};

struct FileSource {
    file: File,
    len: u64,
}

impl ByteSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        #[cfg(unix)]
        let result = std::os::unix::fs::FileExt::read_exact_at(&self.file, buf, offset);
        #[cfg(not(unix))]
        let result = {
            use std::io::{Read, Seek, SeekFrom};
            let mut file = &self.file;
            file.seek(SeekFrom::Start(offset))
                .and_then(|_| file.read_exact(buf))
        };
        result.map_err(|e| ReadError::Backend(Box::new(e)))
    }
}

/// Multiplier of the polynomial row hash.
const P: u64 = 0x9E37_79B9_7F4A_7C15;

/// `sum of row_hash(i) * P^(n - 1 - i)` over the rows, and `P^n`: two batches combine
/// as `(h1 * p2 + h2, p1 * p2)`, so the digest of a file does not depend on where its
/// batches end.
fn digest(batch: &Batch) -> (u64, u64) {
    let (mut h, mut power) = (0_u64, 1_u64);
    for i in 0..batch.len() {
        let mut row = 0xcbf2_9ce4_8422_2325_u64;
        for ids in [&batch.source, &batch.target] {
            let bytes = match ids {
                Ids::Integer(v) => v[i].to_le_bytes().to_vec(),
                Ids::String(s) => s.get(i).unwrap_or_default().to_vec(),
            };
            for b in bytes.into_iter().chain([0xff]) {
                row = (row ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        h = h.wrapping_mul(P).wrapping_add(row);
        power = power.wrapping_mul(P);
    }
    (h, power)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: text_parse FILE [--repeat N] [--digest]");
        std::process::exit(2);
    };
    let repeat: usize = args
        .iter()
        .position(|a| a == "--repeat")
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(3);
    let with_digest = args.iter().any(|a| a == "--digest");
    let file = File::open(path).expect("open the input");
    let len = file.metadata().expect("its length").len();
    let reader =
        TextReader::open(FileSource { file, len }, &TextOptions::default()).expect("open as text");

    let mut seconds = Vec::new();
    let mut summary = None;
    let mut combined = 0_u64;
    for _ in 0..repeat.max(1) {
        let start = Instant::now();
        let out = reader
            .read(|batch| if with_digest { digest(&batch) } else { (0, 1) })
            .unwrap_or_else(|e| panic!("{e}"));
        seconds.push(start.elapsed().as_secs_f64());
        combined = out.batches.iter().fold(0_u64, |acc, &(h, power)| {
            acc.wrapping_mul(power).wrapping_add(h)
        });
        summary = Some(out.summary);
    }
    let summary = summary.expect("at least one pass");
    seconds.sort_by(f64::total_cmp);
    let best = seconds[0];
    let median = seconds[seconds.len() / 2];
    let layout = reader.layout();
    // The plain-text size is the parsed bytes; for a compressed input it is the file.
    #[allow(clippy::cast_precision_loss)]
    let gb_per_s = len as f64 / best / 1e9;
    println!(
        "{{\"input_bytes\":{len},\"compression\":\"{}\",\"ids\":\"{:?}\",\"lines\":{},\"rows\":{},\"invalid_rows\":{},\"repeat\":{},\"seconds_best\":{best:.6},\"seconds_median\":{median:.6},\"input_gb_per_s\":{gb_per_s:.4},\"digest\":{}}}",
        layout.compression,
        layout.ids,
        summary.lines,
        summary.rows,
        summary.invalid_rows,
        seconds.len(),
        if with_digest {
            combined.to_string()
        } else {
            "null".to_owned()
        },
    );
}
