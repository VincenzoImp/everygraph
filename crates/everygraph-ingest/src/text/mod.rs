// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text and CSV edge lists: one edge per line, read through a [`ByteSource`].
//!
//! [`TextReader::open`] looks at the first 64 KiB once and settles the [`Layout`]: the
//! compression (by magic bytes), the delimiter, whether the first line is a header,
//! which columns are the source, target, weight, time, type and extra columns, and
//! whether ids are integers or strings. [`TextReader::read`] then parses the whole
//! input and hands each [`Batch`] of rows to a function, in parallel through
//! [`everygraph_core::par`]; it can be called again for a second pass, because a
//! `ByteSource` is re-readable.
//!
//! - **Plain text** is cut into 4 MiB chunks at page-aligned offsets. A chunk owns the
//!   lines that start in it: it skips to the first line start and reads past its end up
//!   to the end of its last line. Chunk boundaries depend on the offsets only, so the
//!   batches are the same at any thread count.
//! - **gzip** (also several members, as `bgzip` writes) is inflated on one
//!   core, which is the bottleneck: inflate runs at 0.3–0.6 GB/s, 10–20× slower than
//!   parallel parsing (FINDINGS § B1). The command line warns and suggests
//!   decompressing once (`docs/ux-contract.md` § 6.2).
//! - **zstd** files with several frames (as `pzstd` writes them, or several files
//!   concatenated) have their frames decoded in parallel, a group of up to 32 MiB of
//!   compressed bytes at a time. A frame larger than that, as the `zstd` command writes
//!   (one frame per file, also with `-T`), is decoded on one core, at about half the
//!   speed of gzip with the pure-Rust decoder.
//!
//! Decompressed text is parsed in 32 MiB windows, each cut at line ends into 4 MiB
//! pieces that are parsed in parallel. The rows are the same as from the plain file;
//! the batch boundaries are not.
//!
//! # Lines
//!
//! Lines end with `\n` or `\r\n`; the last one may lack it. A UTF-8 byte-order mark is
//! skipped. Empty lines and lines whose first non-blank byte is a comment character (`#`
//! and `%` by default) are skipped. A record cannot span lines: a quoted field with a
//! line break in it is reported as an unclosed quote. A line longer than 1 MiB is an
//! invalid row. Line numbers count from 1 and include the header and comments, as
//! `docs/ux-contract.md` § 6.1 asks.
//!
//! # Fields
//!
//! With a single-byte delimiter other than tab, fields follow RFC 4180: a field that
//! starts with `"` is quoted, `""` inside it is one `"`, and the delimiter may appear in
//! it. Blanks around a field are not part of it. Tab-separated values are not quoted
//! (IANA `text/tab-separated-values`; Wikipedia's clickstream has titles that start
//! with `"`). With [`Delimiter::Whitespace`] fields are separated by runs of spaces and
//! tabs, and there is no quoting either.
//!
//! # Invalid rows
//!
//! The whole input is always parsed and every invalid row counted ([`Summary`]). With
//! [`OnInvalid::Refuse`], the default, any invalid row makes [`TextReader::read`] return
//! [`TextError::InvalidRows`], and whatever the function made of the batches is
//! dropped; with [`OnInvalid::Skip`] the invalid rows are left out and counted. Either
//! way the first [`TextOptions::max_problems`] problems are kept, in line order, with
//! their line number, column, field span and an excerpt of the line.

mod decompress;
mod detect;
mod parse;
mod time;

use std::fmt;
use std::io::Read;
use std::num::NonZeroUsize;
use std::ops::Range;

use everygraph_core::{ByteSource, ReadError, par};

pub use parse::{Ids, Strings};

/// How fields are separated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Delimiter {
    /// Detect it from the first lines: tab, comma, semicolon or `|` if one of them
    /// appears the same number of times on every line looked at, else whitespace.
    #[default]
    Auto,
    /// Runs of spaces and tabs, without quoting (SNAP and most edge lists).
    Whitespace,
    /// One byte, with RFC 4180 quoting unless it is a tab.
    Byte(u8),
}

/// Whether the first line that is not a comment names the columns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Header {
    /// A header if a column is asked for by name; otherwise if the first line has a
    /// field named `from`, `to`, `source`, `target`, `src`, `dst`, `weight` or `time`
    /// (in any case), or if no field of it holds a digit while every following line
    /// looked at has one.
    #[default]
    Auto,
    /// The first line is a header.
    Present,
    /// There is no header.
    Absent,
}

/// A column, by position or by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Column {
    /// Position, from 0.
    Index(usize),
    /// Name in the header. An exact match wins; otherwise the match ignores ASCII case.
    Name(String),
}

impl From<usize> for Column {
    fn from(index: usize) -> Self {
        Self::Index(index)
    }
}

impl From<&str> for Column {
    fn from(name: &str) -> Self {
        Self::Name(name.to_owned())
    }
}

/// How the source and target fields are read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IdType {
    /// [`IdType::Integer`] if every source and target among the first lines is a
    /// decimal integer, else [`IdType::String`].
    #[default]
    Auto,
    /// Decimal integers below 2^64. A larger value is an invalid row, never wrapped.
    Integer,
    /// Hexadecimal integers below 2^64, with or without a `0x` prefix. Wider
    /// hexadecimal keys (addresses, hashes) are read as [`IdType::String`]; the id
    /// dictionary recognizes and packs them.
    Hex,
    /// The bytes of the field, unquoted. Must not be empty.
    String,
}

/// The unit of a time column written as a number since 1970-01-01T00:00:00Z.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeUnit {
    /// Seconds, possibly with a fraction.
    #[default]
    Seconds,
    /// Milliseconds, possibly with a fraction.
    Milliseconds,
    /// Microseconds.
    Microseconds,
}

/// What to do with a row that cannot be read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OnInvalid {
    /// Read the whole input, then refuse it if any row was invalid
    /// (`docs/ux-contract.md` M-6).
    #[default]
    Refuse,
    /// Leave invalid rows out, and count them.
    Skip,
}

/// How the bytes of the source are compressed, recognized by their first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// Plain text.
    None,
    /// gzip. Inflate runs on one core and is 10–20× slower than parsing.
    Gzip,
    /// Zstandard.
    Zstd,
}

impl fmt::Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::None => "plain text",
            Self::Gzip => "gzip",
            Self::Zstd => "zstd",
        })
    }
}

/// Options of [`TextReader::open`]. The default detects everything and refuses
/// invalid rows.
#[derive(Clone, Debug)]
pub struct TextOptions {
    /// How fields are separated.
    pub delimiter: Delimiter,
    /// Whether there is a header line.
    pub header: Header,
    /// Bytes that start a comment line (after leading blanks). Default `#` and `%`.
    pub comments: Vec<u8>,
    /// The source column. Default: the column named `from` or `source` if the header
    /// has the pair `from`/`to` or `source`/`target`, else the first.
    pub source: Option<Column>,
    /// The target column. Default: `to` or `target` as for the source, else the second.
    pub target: Option<Column>,
    /// A weight column, read as a 64-bit float.
    pub weight: Option<Column>,
    /// A time column: a number in [`TextOptions::time_unit`] since the epoch, or ISO
    /// 8601 (`2024-03-02`, `2024-03-02T11:04:35Z`, `2024-03-02 11:04:35.25+01:00`).
    pub time: Option<Column>,
    /// An edge-type column, kept as bytes.
    pub kind: Option<Column>,
    /// Other columns to keep, as bytes, in this order.
    pub extra: Vec<Column>,
    /// How ids are read.
    pub ids: IdType,
    /// The unit of numeric times.
    pub time_unit: TimeUnit,
    /// What to do with invalid rows.
    pub on_invalid: OnInvalid,
    /// How many problems to keep for the report; all are counted.
    pub max_problems: usize,
}

impl Default for TextOptions {
    fn default() -> Self {
        Self {
            delimiter: Delimiter::Auto,
            header: Header::Auto,
            comments: vec![b'#', b'%'],
            source: None,
            target: None,
            weight: None,
            time: None,
            kind: None,
            extra: Vec::new(),
            ids: IdType::Auto,
            time_unit: TimeUnit::Seconds,
            on_invalid: OnInvalid::Refuse,
            max_problems: 10,
        }
    }
}

/// What [`TextReader::open`] settled. Delimiter and id type are never `Auto` here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Compression of the source.
    pub compression: Compression,
    /// [`Delimiter::Whitespace`] or [`Delimiter::Byte`].
    pub delimiter: Delimiter,
    /// The names in the header, if there is one.
    pub header: Option<Vec<String>>,
    /// Position of the source column.
    pub source: usize,
    /// Position of the target column.
    pub target: usize,
    /// Position of the weight column.
    pub weight: Option<usize>,
    /// Position of the time column.
    pub time: Option<usize>,
    /// Position of the edge-type column.
    pub kind: Option<usize>,
    /// Positions of the extra columns, in the order of [`Batch::extra`].
    pub extra: Vec<usize>,
    /// [`IdType::Integer`], [`IdType::Hex`] or [`IdType::String`].
    pub ids: IdType,
}

impl Layout {
    /// The name of a column in messages: its header name, else `column N` (from 1).
    #[must_use]
    pub fn column_name(&self, index: usize) -> String {
        match self.header.as_ref().and_then(|names| names.get(index)) {
            Some(name) => name.clone(),
            None => format!("column {}", index + 1),
        }
    }
}

/// Rows parsed from one part of the input, column by column; all columns have the
/// same length.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    /// Source ids.
    pub source: Ids,
    /// Target ids.
    pub target: Ids,
    /// Weights, if a weight column was chosen.
    pub weight: Option<Vec<f64>>,
    /// Times in microseconds since 1970-01-01T00:00:00Z, if a time column was chosen.
    pub time: Option<Vec<i64>>,
    /// Edge types, if a type column was chosen.
    pub kind: Option<Strings>,
    /// The extra columns, in the order of [`Layout::extra`].
    pub extra: Vec<Strings>,
}

impl Batch {
    /// Number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.source.len()
    }

    /// Whether there are no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Why a line is not a valid row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProblemKind {
    /// The number of fields differs from the header's.
    FieldsNotHeader {
        /// Fields on the line.
        found: usize,
        /// Fields in the header.
        header: usize,
    },
    /// Fewer fields than the highest column asked for needs (no header).
    TooFewFields {
        /// Fields on the line.
        found: usize,
        /// Fields needed.
        needed: usize,
    },
    /// The field is empty.
    Empty,
    /// The field is not a decimal integer.
    NotAnInteger,
    /// The field is not a hexadecimal integer.
    NotHex,
    /// The integer does not fit in 64 bits.
    TooLarge,
    /// The field is not a number.
    NotANumber,
    /// The field is neither a number nor an ISO 8601 date or time.
    NotATime,
    /// The time is too far from 1970 to be held in microseconds.
    TimeOutOfRange,
    /// A quoted field does not end on its line.
    UnclosedQuote,
    /// Something other than the delimiter follows a closing quote.
    TextAfterQuote,
    /// The line is longer than the limit.
    LineTooLong {
        /// The limit, in bytes.
        limit: usize,
    },
}

/// One invalid row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// Line number, from 1, counting every line.
    pub line: u64,
    /// What is wrong.
    pub kind: ProblemKind,
    /// The column the problem is in, named as [`Layout::column_name`] does.
    pub column: Option<String>,
    /// Byte range of the offending field in the line, for carets.
    pub span: Option<Range<usize>>,
    /// The first 160 bytes of the line, without its line ending.
    pub excerpt: Vec<u8>,
}

impl fmt::Display for Problem {
    /// The sentence of `docs/ux-contract.md` § 6.1, without location or full stop:
    /// `5 fields where the header has 7`, `value is not a number`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let column = self.column.as_deref().unwrap_or("the field");
        let fields = |n: usize| {
            if n == 1 {
                "1 field".to_owned()
            } else {
                format!("{n} fields")
            }
        };
        match self.kind {
            ProblemKind::FieldsNotHeader { found, header } => {
                write!(f, "{} where the header has {header}", fields(found))
            }
            ProblemKind::TooFewFields { found, needed } => {
                write!(f, "{} where {needed} are needed", fields(found))
            }
            ProblemKind::Empty => write!(f, "{column} is empty"),
            ProblemKind::NotAnInteger => write!(f, "{column} is not an integer"),
            ProblemKind::NotHex => write!(f, "{column} is not a hexadecimal integer"),
            ProblemKind::TooLarge => write!(f, "{column} is larger than {}", u64::MAX),
            ProblemKind::NotANumber => write!(f, "{column} is not a number"),
            ProblemKind::NotATime => write!(f, "{column} is not a time"),
            ProblemKind::TimeOutOfRange => write!(f, "{column} is too far from 1970"),
            ProblemKind::UnclosedQuote => write!(f, "a quote is not closed"),
            ProblemKind::TextAfterQuote => write!(f, "text follows a closing quote"),
            ProblemKind::LineTooLong { limit } => {
                write!(f, "the line is longer than {limit} bytes")
            }
        }
    }
}

/// Counts of a whole read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Every line, including the header, comments and empty lines.
    pub lines: u64,
    /// Valid rows, handed out in batches.
    pub rows: u64,
    /// Invalid rows.
    pub invalid_rows: u64,
    /// The first [`TextOptions::max_problems`] problems, in line order.
    pub problems: Vec<Problem>,
}

/// The result of [`TextReader::read`].
#[derive(Debug)]
pub struct TextOutput<T> {
    /// What the function returned for each batch, in input order. The function is not
    /// called on empty batches.
    pub batches: Vec<T>,
    /// Counts.
    pub summary: Summary,
}

/// Why an input cannot be read.
#[derive(Debug)]
#[non_exhaustive]
pub enum TextError {
    /// The byte source failed.
    Read(ReadError),
    /// The compressed stream is damaged or truncated.
    Damaged {
        /// Its compression.
        compression: Compression,
        /// What the decoder said.
        reason: String,
    },
    /// A column was asked for by a name the header does not have.
    UnknownColumn {
        /// The name asked for.
        name: String,
        /// The names in the header.
        header: Vec<String>,
    },
    /// A column was asked for by name, and the input has no header.
    NoHeader {
        /// The name asked for.
        name: String,
    },
    /// A column was asked for by a position the header does not have.
    BeyondHeader {
        /// The position asked for, from 0.
        index: usize,
        /// Columns in the header.
        header: usize,
    },
    /// Rows were invalid and [`OnInvalid::Refuse`] was in force.
    InvalidRows(Box<Summary>),
}

impl fmt::Display for TextError {
    /// The part of `Cannot read FILE: …` after the colon, without full stop.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(cause) => write!(f, "{cause}"),
            Self::Damaged {
                compression,
                reason,
            } => write!(f, "the {compression} data is damaged ({reason})"),
            Self::UnknownColumn { name, .. } => write!(f, "it has no column named {name}"),
            Self::NoHeader { name } => {
                write!(f, "it has no header, so there is no column named {name}")
            }
            Self::BeyondHeader { index, header } => write!(
                f,
                "column {} was asked for and the header has {header}",
                index + 1
            ),
            Self::InvalidRows(summary) => match summary.invalid_rows {
                1 => f.write_str("1 line is not a valid row"),
                n => write!(f, "{n} lines are not valid rows"),
            },
        }
    }
}

impl std::error::Error for TextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(cause) => Some(cause),
            _ => None,
        }
    }
}

impl From<ReadError> for TextError {
    fn from(cause: ReadError) -> Self {
        Self::Read(cause)
    }
}

/// Sizes that tests shrink to reach every boundary case with small inputs.
#[derive(Clone, Copy, Debug)]
struct Tuning {
    /// Plain-text chunk; a multiple of the page size in production.
    chunk: usize,
    /// Read past the end of a chunk in steps of this many bytes.
    step: usize,
    /// Decompressed text is parsed this many bytes at a time.
    window: usize,
    /// Pieces of a window parsed in parallel.
    piece: usize,
    /// Longest valid line.
    max_line: usize,
    /// Compressed bytes of zstd frames decoded in parallel at once.
    zstd_group: u64,
}

const TUNING: Tuning = Tuning {
    chunk: 4 << 20,
    step: 64 << 10,
    window: 32 << 20,
    piece: 4 << 20,
    max_line: 1 << 20,
    zstd_group: 32 << 20,
};

/// Bytes looked at by [`TextReader::open`].
const SAMPLE: usize = 64 << 10;

const ONE: NonZeroUsize = NonZeroUsize::MIN;

/// A text edge list ready to be read, once or several times.
#[derive(Debug)]
pub struct TextReader<S> {
    source: S,
    layout: Layout,
    plan: parse::Plan,
    /// Offset, in the decompressed text, of the first line after the header.
    data_start: u64,
    /// Lines before it.
    lines_before: u64,
    on_invalid: OnInvalid,
    max_problems: usize,
    tuning: Tuning,
}

impl<S: ByteSource + Sync> TextReader<S> {
    /// Looks at the start of `source` and settles its [`Layout`].
    ///
    /// # Errors
    ///
    /// [`TextError::Read`] or [`TextError::Damaged`] if the first bytes cannot be read,
    /// and [`TextError::UnknownColumn`], [`TextError::NoHeader`] or
    /// [`TextError::BeyondHeader`] if a column asked for does not exist. An empty
    /// source is not an error: it has no rows.
    pub fn open(source: S, options: &TextOptions) -> Result<Self, TextError> {
        Self::open_tuned(source, options, TUNING)
    }

    fn open_tuned(source: S, options: &TextOptions, tuning: Tuning) -> Result<Self, TextError> {
        let compression = decompress::detect(&source)?;
        let mut sample = Vec::new();
        let complete = {
            let reader = decompress::reader(&source, compression, false, tuning.zstd_group)?;
            let mut limited = reader.take(widen(SAMPLE) + 1);
            decompress::read_all(&mut limited, &mut sample, compression)?;
            sample.len() <= SAMPLE
        };
        sample.truncate(SAMPLE);
        let found = detect::detect(&sample, complete, options)?;
        let layout = Layout {
            compression,
            ..found.layout
        };
        let plan = parse::Plan::new(&layout, options, tuning.max_line);
        Ok(Self {
            source,
            layout,
            plan,
            data_start: found.data_start,
            lines_before: found.lines_before,
            on_invalid: options.on_invalid,
            max_problems: options.max_problems,
            tuning,
        })
    }

    /// What [`open`](Self::open) settled.
    #[must_use]
    pub const fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The source.
    #[must_use]
    pub const fn source(&self) -> &S {
        &self.source
    }

    /// Parses every row and calls `f` on each batch, possibly in parallel; returns what
    /// `f` returned, in input order, and the counts.
    ///
    /// # Errors
    ///
    /// [`TextError::Read`] or [`TextError::Damaged`] if the bytes cannot be read, and
    /// [`TextError::InvalidRows`] under [`OnInvalid::Refuse`] if any row is invalid
    /// (the whole input is read first, so that the count is exact).
    pub fn read<T, F>(&self, f: F) -> Result<TextOutput<T>, TextError>
    where
        T: Send,
        F: Fn(Batch) -> T + Send + Sync,
    {
        let mut out = Collector::new(self.lines_before, self.max_problems);
        match self.layout.compression {
            Compression::None => self.read_plain(&f, &mut out)?,
            compression => {
                let mut reader =
                    decompress::reader(&self.source, compression, true, self.tuning.zstd_group)?;
                decompress::skip(&mut reader, self.data_start, compression)?;
                self.read_stream(&mut reader, compression, &f, &mut out)?;
            }
        }
        let (batches, summary) = out.finish();
        if summary.invalid_rows > 0 && self.on_invalid == OnInvalid::Refuse {
            return Err(TextError::InvalidRows(Box::new(summary)));
        }
        Ok(TextOutput { batches, summary })
    }

    fn read_plain<T, F>(&self, f: &F, out: &mut Collector<T>) -> Result<(), TextError>
    where
        T: Send,
        F: Fn(Batch) -> T + Send + Sync,
    {
        let len = self.source.len();
        let chunk = widen(self.tuning.chunk);
        let first = self.data_start / chunk;
        let count = len.div_ceil(chunk).saturating_sub(first);
        // More chunks than fit in memory cannot be indexed anyway.
        let count = usize::try_from(count).map_err(|_| oversized(len))?;
        let parts = par::map_ranges(count, ONE, |range| {
            let k = first + widen(range.start);
            let start = (k * chunk).max(self.data_start);
            let end = ((k + 1) * chunk).min(len);
            // Taken, not borrowed: `f` may read another source on this thread.
            let mut buf = CHUNK.take();
            let part = self
                .chunk_lines(start, end, &mut buf)
                .map(|(segment, tail)| self.plan.parse(&buf[segment], tail.map(|t| &buf[t])));
            CHUNK.set(buf);
            Ok::<_, TextError>(part?.into_output(f))
        });
        for part in parts {
            out.push(part?);
        }
        Ok(())
    }

    /// Reads the lines that start in `start..end` into `buf`. Returns the range of
    /// `buf` holding whole lines, and the start of a last line too long to read whole.
    fn chunk_lines(
        &self,
        start: u64,
        end: u64,
        buf: &mut Vec<u8>,
    ) -> Result<(Range<usize>, Option<Range<usize>>), ReadError> {
        let len = self.source.len();
        let lead = u64::from(start > self.data_start);
        read_into(&self.source, start - lead, end, buf)?;
        let first = if lead == 0 {
            0
        } else if buf[0] == b'\n' {
            1
        } else {
            match find(b'\n', &buf[1..]) {
                Some(k) => k + 2,
                None => return Ok((0..0, None)),
            }
        };
        if end == len || buf.last() == Some(&b'\n') || first == buf.len() {
            return Ok((first..buf.len(), None));
        }
        let tail = rfind(b'\n', &buf[first..]).map_or(first, |k| first + k + 1);
        let mut at = end;
        loop {
            if buf.len() - tail > self.tuning.max_line {
                return Ok((first..tail, Some(tail..buf.len())));
            }
            let next = (at + widen(self.tuning.step)).min(len);
            let old = buf.len();
            extend_from(&self.source, at, next, buf)?;
            at = next;
            if let Some(k) = find(b'\n', &buf[old..]) {
                buf.truncate(old + k + 1);
                return Ok((first..buf.len(), None));
            }
            if at == len {
                return Ok((first..buf.len(), None));
            }
        }
    }

    fn read_stream<T, F>(
        &self,
        reader: &mut dyn Read,
        compression: Compression,
        f: &F,
        out: &mut Collector<T>,
    ) -> Result<(), TextError>
    where
        T: Send,
        F: Fn(Batch) -> T + Send + Sync,
    {
        let tuning = self.tuning;
        let mut buf: Vec<u8> = Vec::new();
        // Inside a line found too long, whose end has not been read yet.
        let mut skipping = false;
        loop {
            let want = tuning.window.max(buf.len() + tuning.step);
            let before = buf.len();
            let mut limited = (&mut *reader).take(widen(want - before));
            decompress::read_all(&mut limited, &mut buf, compression)?;
            let eof = buf.len() < want;
            if skipping {
                match find(b'\n', &buf[before..]) {
                    Some(k) => {
                        buf.drain(..before + k + 1);
                        skipping = false;
                    }
                    None => {
                        buf.clear();
                        if eof {
                            return Ok(());
                        }
                        continue;
                    }
                }
            }
            let cut = if eof {
                buf.len()
            } else {
                rfind(b'\n', &buf).map_or(0, |k| k + 1)
            };
            if cut == 0 {
                if eof {
                    return Ok(());
                }
                if buf.len() > tuning.max_line {
                    let part = self.plan.parse(&[], Some(&buf));
                    out.push(part.into_output(f));
                    buf.clear();
                    skipping = true;
                }
                continue;
            }
            let pieces = split_lines(&buf[..cut], tuning.piece);
            let parts = par::map_ranges(pieces.len() - 1, ONE, |range| {
                let piece = &buf[pieces[range.start]..pieces[range.start + 1]];
                self.plan.parse(piece, None).into_output(f)
            });
            for part in parts {
                out.push(part);
            }
            buf.drain(..cut);
            if eof && buf.is_empty() {
                return Ok(());
            }
        }
    }
}

/// Boundaries of pieces of about `size` bytes that end at line ends; `text` ends at a
/// line end or at the end of the input.
fn split_lines(text: &[u8], size: usize) -> Vec<usize> {
    let mut bounds = vec![0];
    let mut at = 0;
    while at < text.len() {
        let from = at + size.min(text.len() - at);
        at = if from >= text.len() {
            text.len()
        } else {
            find(b'\n', &text[from - 1..]).map_or(text.len(), |k| from + k)
        };
        bounds.push(at);
    }
    bounds
}

/// The output of one part: what `f` made of its batch, and its counts.
struct PartOutput<T> {
    /// `None` for a part without valid rows: `f` is not called on empty batches.
    value: Option<T>,
    lines: u64,
    rows: u64,
    invalid: u64,
    /// Line numbers from 0, relative to the part.
    problems: Vec<Problem>,
}

impl parse::Partial {
    fn into_output<T>(self, f: &impl Fn(Batch) -> T) -> PartOutput<T> {
        PartOutput {
            rows: widen(self.batch.len()),
            value: (!self.batch.is_empty()).then(|| f(self.batch)),
            lines: self.lines,
            invalid: self.invalid,
            problems: self.problems,
        }
    }
}

/// Puts the parts together in input order: absolute line numbers, totals, and the
/// first problems.
struct Collector<T> {
    batches: Vec<T>,
    summary: Summary,
    max_problems: usize,
}

impl<T> Collector<T> {
    fn new(lines_before: u64, max_problems: usize) -> Self {
        Self {
            batches: Vec::new(),
            summary: Summary {
                lines: lines_before,
                ..Summary::default()
            },
            max_problems,
        }
    }

    fn push(&mut self, part: PartOutput<T>) {
        let base = self.summary.lines + 1;
        let room = self.max_problems - self.summary.problems.len();
        self.summary
            .problems
            .extend(part.problems.into_iter().take(room).map(|mut problem| {
                problem.line += base;
                problem
            }));
        self.summary.lines += part.lines;
        self.summary.rows += part.rows;
        self.summary.invalid_rows += part.invalid;
        self.batches.extend(part.value);
    }

    fn finish(self) -> (Vec<T>, Summary) {
        (self.batches, self.summary)
    }
}

const fn widen(n: usize) -> u64 {
    n as u64
}

fn oversized(len: u64) -> ReadError {
    ReadError::Backend(format!("a source of {len} bytes is too large for this platform").into())
}

std::thread_local! {
    /// Each thread's chunk buffer, kept between chunks: a fresh one costs a page fault
    /// every 4 KiB. It stays allocated (one chunk and a little more) after a read.
    static CHUNK: std::cell::Cell<Vec<u8>> = const { std::cell::Cell::new(Vec::new()) };
}

/// Replaces the contents of `buf` with the bytes `start..end` of `source`.
fn read_into(
    source: &impl ByteSource,
    start: u64,
    end: u64,
    buf: &mut Vec<u8>,
) -> Result<(), ReadError> {
    let n = usize::try_from(end - start).map_err(|_| oversized(end - start))?;
    // Resized, not cleared: the bytes are overwritten, so only growth is zeroed.
    buf.resize(n, 0);
    source.read_at(start, buf)
}

fn extend_from(
    source: &impl ByteSource,
    start: u64,
    end: u64,
    buf: &mut Vec<u8>,
) -> Result<(), ReadError> {
    let n = usize::try_from(end - start).map_err(|_| oversized(end - start))?;
    let old = buf.len();
    buf.resize(old + n, 0);
    source.read_at(start, &mut buf[old..])
}

fn find(byte: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == byte)
}

fn rfind(byte: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().rposition(|&b| b == byte)
}

#[cfg(test)]
mod tests;
