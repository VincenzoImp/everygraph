// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lines to rows: field splitting, typed fields, and the columns of a [`Batch`].

use std::ops::Range;

use super::{
    Batch, Delimiter, IdType, Layout, Problem, ProblemKind, TextOptions, TimeUnit, find, time,
};

/// Bytes of a line kept as the excerpt of a problem.
const EXCERPT: usize = 160;

/// Ids of a column of a [`Batch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ids {
    /// Integer ids ([`IdType::Integer`] and [`IdType::Hex`]).
    Integer(Vec<u64>),
    /// The bytes of each id ([`IdType::String`]).
    String(Strings),
}

impl Ids {
    /// Number of ids.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Integer(ids) => ids.len(),
            Self::String(ids) => ids.len(),
        }
    }

    /// Whether there are no ids.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn truncate(&mut self, len: usize) {
        match self {
            Self::Integer(ids) => ids.truncate(len),
            Self::String(ids) => ids.truncate(len),
        }
    }
}

/// A column of byte strings, stored end to end.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Strings {
    bytes: Vec<u8>,
    ends: Vec<usize>,
}

impl Strings {
    /// Number of strings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    /// Whether there are no strings.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// String `i`.
    #[must_use]
    pub fn get(&self, i: usize) -> Option<&[u8]> {
        let end = *self.ends.get(i)?;
        let start = if i == 0 { 0 } else { self.ends[i - 1] };
        Some(&self.bytes[start..end])
    }

    /// The strings in order.
    pub fn iter(&self) -> impl Iterator<Item = &[u8]> {
        (0..self.len()).filter_map(|i| self.get(i))
    }

    /// Appends the value of `field` of `line`, turning `""` into `"` in a quoted field.
    fn push(&mut self, line: &[u8], field: Field) {
        let value = &line[field.start..field.end];
        if field.escaped {
            let mut quote = false;
            for &b in value {
                if b == b'"' && quote {
                    quote = false;
                    continue;
                }
                quote = b == b'"';
                self.bytes.push(b);
            }
        } else {
            self.bytes.extend_from_slice(value);
        }
        self.ends.push(self.bytes.len());
    }

    fn truncate(&mut self, len: usize) {
        if len < self.ends.len() {
            self.ends.truncate(len);
            self.bytes.truncate(self.ends.last().copied().unwrap_or(0));
        }
    }
}

/// Position of a field in its line; for a quoted field, of the text between the quotes.
#[derive(Clone, Copy, Debug)]
pub(super) struct Field {
    start: usize,
    end: usize,
    /// Quoted, with `""` inside.
    escaped: bool,
}

impl Field {
    pub(super) fn value(self, line: &[u8]) -> &[u8] {
        &line[self.start..self.end]
    }
}

const fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Whether a line holds no row: blank, or a comment.
pub(super) fn is_skippable(line: &[u8], comments: &[u8]) -> bool {
    line.iter()
        .find(|&&b| !is_blank(b))
        .is_none_or(|b| comments.contains(b))
}

/// Splits `line` into `out` (whitespace when `delimiter` is `None`).
pub(super) fn split_fields(
    line: &[u8],
    delimiter: Option<u8>,
    out: &mut Vec<Field>,
) -> Result<(), (ProblemKind, Range<usize>)> {
    let n = line.len();
    let Some(d) = delimiter else {
        let mut i = 0;
        loop {
            while i < n && is_blank(line[i]) {
                i += 1;
            }
            if i == n {
                return Ok(());
            }
            let start = i;
            while i < n && !is_blank(line[i]) {
                i += 1;
            }
            out.push(Field {
                start,
                end: i,
                escaped: false,
            });
        }
    };
    let blank = |b: u8| is_blank(b) && b != d;
    let mut i = 0;
    loop {
        let mut s = i;
        while s < n && blank(line[s]) {
            s += 1;
        }
        if d != b'\t' && line.get(s) == Some(&b'"') {
            let mut j = s + 1;
            let mut escaped = false;
            let close = loop {
                match find(b'"', &line[j..]) {
                    None => return Err((ProblemKind::UnclosedQuote, s..n)),
                    Some(k) if line.get(j + k + 1) == Some(&b'"') => {
                        escaped = true;
                        j += k + 2;
                    }
                    Some(k) => break j + k,
                }
            };
            out.push(Field {
                start: s + 1,
                end: close,
                escaped,
            });
            let mut k = close + 1;
            while k < n && blank(line[k]) {
                k += 1;
            }
            if k == n {
                return Ok(());
            }
            if line[k] != d {
                return Err((ProblemKind::TextAfterQuote, s..k + 1));
            }
            i = k + 1;
        } else {
            let e = find(d, &line[i..]).map_or(n, |k| i + k);
            let mut t = e;
            while t > s && blank(line[t - 1]) {
                t -= 1;
            }
            out.push(Field {
                start: s,
                end: t,
                escaped: false,
            });
            if e == n {
                return Ok(());
            }
            i = e + 1;
        }
    }
}

/// A decimal integer below 2^64.
pub(super) fn decimal(b: &[u8]) -> Result<u64, ProblemKind> {
    if b.is_empty() {
        return Err(ProblemKind::Empty);
    }
    let mut value = 0_u64;
    // Nineteen digits are below 10^19 < 2^64: no check needed.
    if b.len() <= 19 {
        for &c in b {
            let d = c.wrapping_sub(b'0');
            if d > 9 {
                return Err(ProblemKind::NotAnInteger);
            }
            value = value * 10 + u64::from(d);
        }
        return Ok(value);
    }
    let mut overflow = false;
    for &c in b {
        let d = c.wrapping_sub(b'0');
        if d > 9 {
            return Err(ProblemKind::NotAnInteger);
        }
        match value
            .checked_mul(10)
            .and_then(|v| v.checked_add(u64::from(d)))
        {
            Some(v) => value = v,
            None => overflow = true,
        }
    }
    if overflow {
        Err(ProblemKind::TooLarge)
    } else {
        Ok(value)
    }
}

fn hex(b: &[u8]) -> Result<u64, ProblemKind> {
    if b.is_empty() {
        return Err(ProblemKind::Empty);
    }
    let digits = b
        .strip_prefix(b"0x")
        .or_else(|| b.strip_prefix(b"0X"))
        .unwrap_or(b);
    if digits.is_empty() {
        return Err(ProblemKind::NotHex);
    }
    let mut value = 0_u64;
    let mut overflow = false;
    for &c in digits {
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return Err(ProblemKind::NotHex),
        };
        match value.checked_mul(16) {
            Some(v) => value = v | u64::from(d),
            None => overflow = true,
        }
    }
    if overflow {
        Err(ProblemKind::TooLarge)
    } else {
        Ok(value)
    }
}

fn float(b: &[u8]) -> Result<f64, ProblemKind> {
    if b.is_empty() {
        return Err(ProblemKind::Empty);
    }
    std::str::from_utf8(b)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or(ProblemKind::NotANumber)
}

#[derive(Clone, Copy, Debug)]
enum Expected {
    /// As many fields as the header.
    Exactly(usize),
    /// At least as many as the highest column used needs.
    AtLeast(usize),
}

/// What one part of the input gave.
#[derive(Debug)]
pub(super) struct Partial {
    pub(super) batch: Batch,
    pub(super) lines: u64,
    pub(super) invalid: u64,
    /// Line numbers from 0, relative to the part.
    pub(super) problems: Vec<Problem>,
}

impl Partial {
    fn reject(
        &mut self,
        line: u64,
        kind: ProblemKind,
        column: Option<String>,
        span: Option<Range<usize>>,
        text: &[u8],
    ) {
        self.invalid += 1;
        self.problems.push(Problem {
            line,
            kind,
            column,
            span,
            excerpt: text[..text.len().min(EXCERPT)].to_vec(),
        });
    }
}

/// A [`Layout`] compiled for the parsing loop.
#[derive(Debug)]
pub(super) struct Plan {
    delimiter: Option<u8>,
    comments: Vec<u8>,
    expected: Expected,
    ids: IdType,
    time_unit: TimeUnit,
    source: usize,
    target: usize,
    weight: Option<usize>,
    time: Option<usize>,
    kind: Option<usize>,
    extra: Vec<usize>,
    /// Message names of columns `0..=` the highest used.
    names: Vec<String>,
    max_line: usize,
    /// Keep at most this many problems per part (all are counted).
    max_problems: usize,
}

/// A rejected field: what, in which column, where.
type Reject = (ProblemKind, Option<usize>, Option<Range<usize>>);

impl Plan {
    pub(super) fn new(layout: &Layout, options: &TextOptions, max_line: usize) -> Self {
        let used = [layout.source, layout.target]
            .into_iter()
            .chain(layout.weight)
            .chain(layout.time)
            .chain(layout.kind)
            .chain(layout.extra.iter().copied());
        let highest = used.max().unwrap_or(0);
        let expected = match &layout.header {
            Some(names) => Expected::Exactly(names.len()),
            None => Expected::AtLeast(highest + 1),
        };
        Self {
            delimiter: match layout.delimiter {
                Delimiter::Byte(d) => Some(d),
                Delimiter::Auto | Delimiter::Whitespace => None,
            },
            comments: options.comments.clone(),
            expected,
            ids: layout.ids,
            time_unit: options.time_unit,
            source: layout.source,
            target: layout.target,
            weight: layout.weight,
            time: layout.time,
            kind: layout.kind,
            extra: layout.extra.clone(),
            names: (0..=highest).map(|i| layout.column_name(i)).collect(),
            max_line,
            max_problems: options.max_problems,
        }
    }

    fn empty_batch(&self, capacity: usize) -> Batch {
        let ids = || match self.ids {
            IdType::String => Ids::String(Strings::default()),
            _ => Ids::Integer(Vec::with_capacity(capacity)),
        };
        Batch {
            source: ids(),
            target: ids(),
            weight: self.weight.map(|_| Vec::with_capacity(capacity)),
            time: self.time.map(|_| Vec::with_capacity(capacity)),
            kind: self.kind.map(|_| Strings::default()),
            extra: self.extra.iter().map(|_| Strings::default()).collect(),
        }
    }

    /// Parses `text`, whole lines the last of which may lack its line end, then counts
    /// `overlong` as one more line, too long to have been read whole.
    pub(super) fn parse(&self, text: &[u8], overlong: Option<&[u8]>) -> Partial {
        // Edge lines of SNAP files average about 14–15 bytes (an estimate); the room is
        // reserved, and only touched when used.
        let mut part = Partial {
            batch: self.empty_batch(text.len() / 8),
            lines: 0,
            invalid: 0,
            problems: Vec::new(),
        };
        let mut fields = Vec::with_capacity(8);
        let mut rest = text;
        while !rest.is_empty() {
            let (raw, next) = match find(b'\n', rest) {
                Some(k) => (&rest[..k], &rest[k + 1..]),
                None => (rest, &rest[rest.len()..]),
            };
            rest = next;
            let line_no = part.lines;
            part.lines += 1;
            if raw.len() > self.max_line {
                self.reject(
                    &mut part,
                    line_no,
                    (
                        ProblemKind::LineTooLong {
                            limit: self.max_line,
                        },
                        None,
                        None,
                    ),
                    &raw[..self.max_line],
                );
                continue;
            }
            let line = raw.strip_suffix(b"\r").unwrap_or(raw);
            if is_skippable(line, &self.comments) {
                continue;
            }
            let rows = part.batch.len();
            if let Err(reject) = self.row(line, &mut fields, &mut part.batch) {
                truncate(&mut part.batch, rows);
                self.reject(&mut part, line_no, reject, line);
            }
        }
        if let Some(text) = overlong {
            let line_no = part.lines;
            part.lines += 1;
            self.reject(
                &mut part,
                line_no,
                (
                    ProblemKind::LineTooLong {
                        limit: self.max_line,
                    },
                    None,
                    None,
                ),
                &text[..text.len().min(self.max_line)],
            );
        }
        part
    }

    fn reject(&self, part: &mut Partial, line: u64, (kind, column, span): Reject, text: &[u8]) {
        if part.problems.len() < self.max_problems {
            let column = column.map(|i| self.names[i].clone());
            part.reject(line, kind, column, span, text);
        } else {
            part.invalid += 1;
        }
    }

    /// Appends the row of `line` to `batch`; on error, may leave part of it there.
    fn row(&self, line: &[u8], fields: &mut Vec<Field>, batch: &mut Batch) -> Result<(), Reject> {
        fields.clear();
        split_fields(line, self.delimiter, fields)
            .map_err(|(kind, span)| (kind, None, Some(span)))?;
        let found = fields.len();
        match self.expected {
            Expected::Exactly(header) if found != header => {
                return Err((ProblemKind::FieldsNotHeader { found, header }, None, None));
            }
            Expected::AtLeast(needed) if found < needed => {
                return Err((ProblemKind::TooFewFields { found, needed }, None, None));
            }
            _ => {}
        }
        let at = |column: usize, kind: ProblemKind| {
            let field = fields[column];
            (kind, Some(column), Some(field.start..field.end))
        };
        self.id(line, fields[self.source], &mut batch.source)
            .map_err(|k| at(self.source, k))?;
        self.id(line, fields[self.target], &mut batch.target)
            .map_err(|k| at(self.target, k))?;
        if let (Some(c), Some(out)) = (self.weight, batch.weight.as_mut()) {
            out.push(float(fields[c].value(line)).map_err(|k| at(c, k))?);
        }
        if let (Some(c), Some(out)) = (self.time, batch.time.as_mut()) {
            out.push(time::parse(fields[c].value(line), self.time_unit).map_err(|k| at(c, k))?);
        }
        if let (Some(c), Some(out)) = (self.kind, batch.kind.as_mut()) {
            out.push(line, fields[c]);
        }
        for (&c, out) in self.extra.iter().zip(&mut batch.extra) {
            out.push(line, fields[c]);
        }
        Ok(())
    }

    fn id(&self, line: &[u8], field: Field, out: &mut Ids) -> Result<(), ProblemKind> {
        match out {
            Ids::Integer(ids) => ids.push(match self.ids {
                IdType::Hex => hex(field.value(line))?,
                _ => decimal(field.value(line))?,
            }),
            Ids::String(ids) => {
                if field.start == field.end {
                    return Err(ProblemKind::Empty);
                }
                ids.push(line, field);
            }
        }
        Ok(())
    }
}

/// Cuts every column of `batch` back to `rows` rows.
fn truncate(batch: &mut Batch, rows: usize) {
    batch.source.truncate(rows);
    batch.target.truncate(rows);
    if let Some(column) = batch.weight.as_mut() {
        column.truncate(rows);
    }
    if let Some(column) = batch.time.as_mut() {
        column.truncate(rows);
    }
    if let Some(column) = batch.kind.as_mut() {
        column.truncate(rows);
    }
    for column in &mut batch.extra {
        column.truncate(rows);
    }
}
