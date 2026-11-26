// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Compressed sources as byte streams: gzip on one core, zstd frames in parallel.

use std::io::{self, Read};
use std::ops::Range;

use everygraph_core::{ByteSource, ReadError, par};
use flate2::read::MultiGzDecoder;
use ruzstd::decoding::StreamingDecoder;

use super::{Compression, ONE, TextError, read_into, widen};

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
const ZSTD_MAGIC: u32 = 0xFD2F_B528;
/// Skippable frames have magic numbers `0x184D2A50..=0x184D2A5F`.
const SKIPPABLE_MASK: u32 = 0xFFFF_FFF0;
const SKIPPABLE_MAGIC: u32 = 0x184D_2A50;

/// Bytes a [`SourceReader`] reads at a time.
const READ_BLOCK: usize = 1 << 20;

/// The compression of `source`, from its first four bytes.
pub(super) fn detect(source: &impl ByteSource) -> Result<Compression, ReadError> {
    let mut head = [0_u8; 4];
    let n = usize::try_from(source.len().min(4)).unwrap_or(4);
    source.read_at(0, &mut head[..n])?;
    let magic = u32::from_le_bytes(head);
    Ok(if n >= 2 && head[..2] == GZIP_MAGIC {
        Compression::Gzip
    } else if n == 4 && (magic == ZSTD_MAGIC || magic & SKIPPABLE_MASK == SKIPPABLE_MAGIC) {
        Compression::Zstd
    } else {
        Compression::None
    })
}

/// The decompressed bytes of `source`. With `parallel`, zstd frames are decoded in
/// parallel, a group of up to `group` compressed bytes at a time.
pub(super) fn reader<'a, S: ByteSource + Sync>(
    source: &'a S,
    compression: Compression,
    parallel: bool,
    group: u64,
) -> Result<Box<dyn Read + 'a>, TextError> {
    let raw = SourceReader::new(source, 0..source.len());
    Ok(match compression {
        Compression::None => Box::new(raw),
        Compression::Gzip => Box::new(MultiGzDecoder::new(raw)),
        Compression::Zstd => Box::new(ZstdReader {
            source,
            next: 0,
            group: if parallel { group } else { 0 },
            ready: Vec::new(),
            at: 0,
            stream: None,
            stream_next: false,
        }),
    })
}

/// Appends everything `reader` yields to `buf`.
pub(super) fn read_all(
    reader: &mut dyn Read,
    buf: &mut Vec<u8>,
    compression: Compression,
) -> Result<(), TextError> {
    reader
        .read_to_end(buf)
        .map(|_| ())
        .map_err(|e| error(e, compression))
}

/// Reads at least one byte into `buf` unless the stream has ended; returns how many.
pub(super) fn read_some(
    reader: &mut dyn Read,
    buf: &mut [u8],
    compression: Compression,
) -> Result<usize, TextError> {
    loop {
        match reader.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            other => return other.map_err(|e| error(e, compression)),
        }
    }
}

/// Reads and drops `n` bytes.
pub(super) fn skip(
    reader: &mut dyn Read,
    n: u64,
    compression: Compression,
) -> Result<(), TextError> {
    io::copy(&mut reader.take(n), &mut io::sink())
        .map(|_| ())
        .map_err(|e| error(e, compression))
}

/// A read error of the source, carried through `io::Error`, or a damaged stream.
fn error(e: io::Error, compression: Compression) -> TextError {
    let text = e.to_string();
    match e
        .into_inner()
        .map(|inner: Box<dyn std::error::Error + Send + Sync>| inner.downcast::<ReadError>())
    {
        Some(Ok(cause)) => TextError::Read(*cause),
        _ => damaged(compression, text),
    }
}

fn damaged(compression: Compression, reason: impl Into<String>) -> TextError {
    TextError::Damaged {
        compression,
        reason: reason.into(),
    }
}

/// `std::io::Read` over a range of a [`ByteSource`].
struct SourceReader<'a, S> {
    source: &'a S,
    next: u64,
    end: u64,
    buf: Vec<u8>,
    at: usize,
}

impl<'a, S: ByteSource> SourceReader<'a, S> {
    const fn new(source: &'a S, range: Range<u64>) -> Self {
        Self {
            source,
            next: range.start,
            end: range.end,
            buf: Vec::new(),
            at: 0,
        }
    }
}

impl<S: ByteSource> Read for SourceReader<'_, S> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.at == self.buf.len() {
            let to = self.end.min(self.next + widen(READ_BLOCK));
            if to == self.next {
                return Ok(0);
            }
            read_into(self.source, self.next, to, &mut self.buf).map_err(io::Error::other)?;
            self.next = to;
            self.at = 0;
        }
        let n = out.len().min(self.buf.len() - self.at);
        out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// Decodes the frames of a zstd source in order.
struct ZstdReader<'a, S: ByteSource> {
    source: &'a S,
    /// Offset of the first frame not yet decoded.
    next: u64,
    /// Compressed bytes decoded in parallel at once; 0 streams every frame.
    group: u64,
    ready: Vec<u8>,
    at: usize,
    stream: Option<StreamingDecoder<SourceReader<'a, S>, ruzstd::decoding::FrameDecoder>>,
    /// The frame at `next` expanded past its cap in a group: stream it.
    stream_next: bool,
}

impl<S: ByteSource + Sync> Read for ZstdReader<'_, S> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.at < self.ready.len() {
                let n = out.len().min(self.ready.len() - self.at);
                out[..n].copy_from_slice(&self.ready[self.at..self.at + n]);
                self.at += n;
                return Ok(n);
            }
            if let Some(stream) = self.stream.as_mut() {
                let n = stream.read(out)?;
                if n > 0 {
                    return Ok(n);
                }
                check_checksum(&stream.decoder)?;
                self.stream = None;
                continue;
            }
            if self.next >= self.source.len() {
                return Ok(0);
            }
            self.refill()?;
        }
    }
}

/// Where a frame ends.
enum Frame {
    /// A frame that holds no data.
    Skippable(u64),
    /// A frame of data.
    Data(u64),
    /// A frame of data larger than the budget given.
    Larger,
}

impl<S: ByteSource + Sync> ZstdReader<'_, S> {
    /// Decodes the next group of frames into `ready`, or starts streaming the next
    /// frame when it alone is larger than a group, or expands more than
    /// [`expansion_cap`] allows: the memory a group takes is bounded by its compressed
    /// size, whatever the frames contain.
    fn refill(&mut self) -> Result<(), FrameError> {
        let len = self.source.len();
        if std::mem::take(&mut self.stream_next) {
            return self.start_stream(self.next);
        }
        let mut frames = Vec::new();
        let mut at = self.next;
        let mut budget = self.group;
        while at < len {
            match frame_end(self.source, at, budget)? {
                Frame::Skippable(end) => at = end,
                Frame::Data(end) => {
                    frames.push(at..end);
                    budget -= end - at;
                    at = end;
                }
                Frame::Larger => break,
            }
        }
        if frames.is_empty() {
            if at >= len {
                self.next = at;
                return Ok(());
            }
            return self.start_stream(at);
        }
        let decoded = par::map_ranges(frames.len(), ONE, |range| {
            decode_frame(self.source, frames[range.start].clone())
        });
        self.ready.clear();
        self.at = 0;
        self.next = at;
        for (frame, range) in decoded.into_iter().zip(&frames) {
            if let Some(content) = frame? {
                self.ready.extend_from_slice(&content);
            } else {
                // Streamed next, and the frames after it decoded again.
                self.next = range.start;
                self.stream_next = true;
                break;
            }
        }
        Ok(())
    }

    /// Streams the data frame at `at` (after any skippable frames), on one core.
    fn start_stream(&mut self, mut at: u64) -> Result<(), FrameError> {
        let end = loop {
            match frame_end(self.source, at, u64::MAX)? {
                Frame::Skippable(end) => at = end,
                Frame::Data(end) => break end,
                Frame::Larger => unreachable!("no frame is larger than u64::MAX"),
            }
        };
        let decoder = StreamingDecoder::new(SourceReader::new(self.source, at..end))
            .map_err(|e| FrameError::Damaged(e.to_string()))?;
        self.stream = Some(decoder);
        self.next = end;
        Ok(())
    }
}

/// Why the frames of a zstd source cannot be decoded.
#[derive(Debug)]
enum FrameError {
    Read(ReadError),
    Damaged(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(e) => write!(f, "{e}"),
            Self::Damaged(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<ReadError> for FrameError {
    fn from(e: ReadError) -> Self {
        Self::Read(e)
    }
}

impl From<FrameError> for io::Error {
    fn from(e: FrameError) -> Self {
        match e {
            FrameError::Read(cause) => Self::other(cause),
            FrameError::Damaged(reason) => Self::new(io::ErrorKind::InvalidData, reason),
        }
    }
}

fn check_checksum(decoder: &ruzstd::decoding::FrameDecoder) -> Result<(), FrameError> {
    match (
        decoder.get_checksum_from_data(),
        decoder.get_calculated_checksum(),
    ) {
        (Some(stored), Some(computed)) if stored != computed => {
            Err(FrameError::Damaged("checksum mismatch".to_owned()))
        }
        _ => Ok(()),
    }
}

/// Most bytes a frame of `compressed` bytes may decode to in a parallel group. Text
/// compresses about 3–10× (an estimate); a frame that expands more is streamed instead.
const fn expansion_cap(compressed: u64) -> u64 {
    16 * compressed + (64 << 10)
}

/// The content of the frame at `range`, or `None` if it exceeds [`expansion_cap`].
fn decode_frame(
    source: &impl ByteSource,
    range: Range<u64>,
) -> Result<Option<Vec<u8>>, FrameError> {
    let cap = expansion_cap(range.end - range.start);
    let mut compressed = Vec::new();
    read_into(source, range.start, range.end, &mut compressed)?;
    let mut decoder = StreamingDecoder::new(compressed.as_slice())
        .map_err(|e| FrameError::Damaged(e.to_string()))?;
    let mut out = Vec::new();
    (&mut decoder)
        .take(cap + 1)
        .read_to_end(&mut out)
        .map_err(|e| FrameError::Damaged(e.to_string()))?;
    if widen(out.len()) > cap {
        return Ok(None);
    }
    check_checksum(&decoder.decoder)?;
    Ok(Some(out))
}

fn read_bytes<const N: usize>(source: &impl ByteSource, at: u64) -> Result<[u8; N], FrameError> {
    let mut bytes = [0_u8; N];
    if at
        .checked_add(widen(N))
        .is_none_or(|end| end > source.len())
    {
        return Err(FrameError::Damaged(
            "the data ends inside a frame".to_owned(),
        ));
    }
    source.read_at(at, &mut bytes)?;
    Ok(bytes)
}

/// Walks the headers of the frame at `at` (RFC 8878 § 3.1) to find its end, reading a
/// few bytes per block; gives up with [`Frame::Larger`] past `budget` bytes.
fn frame_end(source: &impl ByteSource, at: u64, budget: u64) -> Result<Frame, FrameError> {
    let magic = u32::from_le_bytes(read_bytes::<4>(source, at)?);
    if magic & SKIPPABLE_MASK == SKIPPABLE_MAGIC {
        let size = u32::from_le_bytes(read_bytes::<4>(source, at + 4)?);
        let end = at + 8 + u64::from(size);
        if end > source.len() {
            return Err(FrameError::Damaged(
                "the data ends inside a frame".to_owned(),
            ));
        }
        return Ok(Frame::Skippable(end));
    }
    if magic != ZSTD_MAGIC {
        return Err(FrameError::Damaged(
            "bytes that are not a zstd frame follow a frame".to_owned(),
        ));
    }
    let [descriptor] = read_bytes::<1>(source, at + 4)?;
    let single_segment = (descriptor >> 5) & 1 == 1;
    let header = 1
        + u64::from(!single_segment)
        + [0, 1, 2, 4][usize::from(descriptor & 3)]
        + match descriptor >> 6 {
            0 => u64::from(single_segment),
            1 => 2,
            2 => 4,
            _ => 8,
        };
    let mut pos = at + 4 + header;
    loop {
        let [b0, b1, b2] = read_bytes::<3>(source, pos)?;
        let block = u32::from_le_bytes([b0, b1, b2, 0]);
        let size = u64::from(block >> 3);
        pos += 3 + match (block >> 1) & 3 {
            1 => 1,
            3 => {
                return Err(FrameError::Damaged(
                    "a zstd block of reserved type".to_owned(),
                ));
            }
            _ => size,
        };
        if pos - at > budget {
            return Ok(Frame::Larger);
        }
        if block & 1 == 1 {
            break;
        }
    }
    if (descriptor >> 2) & 1 == 1 {
        pos += 4;
    }
    if pos - at > budget {
        return Ok(Frame::Larger);
    }
    if pos > source.len() {
        return Err(FrameError::Damaged(
            "the data ends inside a frame".to_owned(),
        ));
    }
    Ok(Frame::Data(pos))
}
