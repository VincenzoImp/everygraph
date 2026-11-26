// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The text reader on untrusted bytes (issue #191): it must never panic, and a frame's
//! declared size must not become an allocation (`docs/research/sota-testing.md`,
//! challenge 10). Run with `-malloc_limit_mb=64` (tools/fuzz.sh does), so that one
//! allocation of a zstd frame's 128 MiB window fails the run.
//!
//! The first byte picks the mode:
//! - 0: the rest is the input, plain or compressed as its magic says;
//! - 1: the rest is wrapped in a gzip member;
//! - 2: the rest is cut into up to four zstd frames of raw blocks, each declaring the
//!   window that the next byte encodes (`0x88` is 128 MiB, the largest the decoder
//!   accepts).

#![no_main]

use std::io::Write;

use everygraph_core::MemorySource;
use everygraph_ingest::text::{TextOptions, TextReader};
use libfuzzer_sys::fuzz_target;

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// zstd frames (RFC 8878 § 3.1.1): magic, a frame header with a window descriptor and
/// no content size, then raw blocks of at most 255 bytes, the last one marked.
fn zstd(bytes: &[u8]) -> Vec<u8> {
    let Some((&shape, rest)) = bytes.split_first() else {
        return Vec::new();
    };
    let frames = usize::from(shape % 4) + 1;
    let mut out = Vec::new();
    for (i, piece) in rest.chunks(rest.len().div_ceil(frames).max(1)).enumerate() {
        let Some((&window, content)) = piece.split_first() else {
            continue;
        };
        out.extend_from_slice(&[0x28, 0xb5, 0x2f, 0xfd, 0x00, window]);
        let blocks: Vec<&[u8]> = content.chunks(255).collect();
        for (j, block) in blocks.iter().enumerate() {
            let last = u32::from(j + 1 == blocks.len());
            let header = last | (block.len() as u32) << 3;
            out.extend_from_slice(&header.to_le_bytes()[..3]);
            out.extend_from_slice(block);
        }
        if blocks.is_empty() {
            out.extend_from_slice(&[1, 0, 0]);
        }
        // A skippable frame between some frames, as `pzstd` writes.
        if i % 2 == 1 {
            out.extend_from_slice(&[0x50, 0x2a, 0x4d, 0x18, 0, 0, 0, 0]);
        }
    }
    out
}

fuzz_target!(|bytes: &[u8]| {
    let Some((&mode, rest)) = bytes.split_first() else {
        return;
    };
    let input = match mode % 3 {
        0 => rest.to_vec(),
        1 => gzip(rest),
        _ => zstd(rest),
    };
    let Ok(reader) = TextReader::open(MemorySource::new(input), &TextOptions::default()) else {
        return;
    };
    // A second pass must give the same rows: a `ByteSource` is re-readable.
    let first = reader.read(|batch| batch.len()).map(|o| o.summary.rows);
    let second = reader.read(|batch| batch.len()).map(|o| o.summary.rows);
    assert_eq!(first.ok(), second.ok());
});
