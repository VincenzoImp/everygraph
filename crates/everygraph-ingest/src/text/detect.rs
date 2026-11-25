// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The layout of an input, from its first bytes.

use super::parse::{Field, decimal, is_skippable, split_fields};
use super::{
    Column, Compression, Delimiter, Header, IdType, Layout, TextError, TextOptions, find, widen,
};

/// Lines looked at for the delimiter, the header and the id type.
const LINES: usize = 100;

/// Header names that make the first line a header.
const KNOWN: [&str; 8] = [
    "from", "to", "source", "target", "src", "dst", "weight", "time",
];

pub(super) struct Found {
    pub(super) layout: Layout,
    /// Offset of the first line after the header (after a byte-order mark if none).
    pub(super) data_start: u64,
    pub(super) lines_before: u64,
}

/// A line of the sample: where the next line starts, and its text without line end.
struct Line<'a> {
    next: usize,
    text: &'a [u8],
}

/// Settles the layout from `sample`, the first bytes of the decompressed text
/// (`complete` if it is all of it).
pub(super) fn detect(
    sample: &[u8],
    complete: bool,
    options: &TextOptions,
) -> Result<Found, TextError> {
    let bom = if sample.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    let body = &sample[bom..];
    let usable = if complete {
        body.len()
    } else {
        body.iter().rposition(|&b| b == b'\n').map_or(0, |k| k + 1)
    };
    let mut lines = Vec::new();
    let mut at = 0;
    while at < usable {
        let end = find(b'\n', &body[at..usable]).map_or(usable, |k| at + k);
        let text = &body[at..end];
        lines.push(Line {
            next: (end + 1).min(usable),
            text: text.strip_suffix(b"\r").unwrap_or(text),
        });
        at = end + 1;
    }
    // (index of the line, its text) for lines that may hold a row.
    let content: Vec<(usize, &[u8])> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !is_skippable(line.text, &options.comments))
        .map(|(i, line)| (i, line.text))
        .take(LINES)
        .collect();

    let delimiter = match options.delimiter {
        Delimiter::Auto => guess_delimiter(&content),
        given => given,
    };
    let byte = match delimiter {
        Delimiter::Byte(d) => Some(d),
        Delimiter::Auto | Delimiter::Whitespace => None,
    };
    let named = [
        &options.source,
        &options.target,
        &options.weight,
        &options.time,
        &options.kind,
    ]
    .into_iter()
    .flatten()
    .chain(&options.extra)
    .any(|c| matches!(c, Column::Name(_)));
    let has_header = !content.is_empty()
        && match options.header {
            Header::Present => true,
            Header::Absent => false,
            Header::Auto => named || looks_like_header(&content, byte),
        };
    let (header, data_start, lines_before, rows) = if has_header {
        let (index, text) = content[0];
        let names = field_texts(text, byte);
        (
            Some(names),
            bom + lines[index].next,
            widen(index + 1),
            &content[1..],
        )
    } else {
        (None, bom, 0, &content[..])
    };

    let resolve = |column: &Column| resolve(column, header.as_deref());
    let (source, target) = match (&options.source, &options.target) {
        (None, None) => default_pair(header.as_deref()),
        (source, target) => (
            source.clone().unwrap_or(Column::Index(0)),
            target.clone().unwrap_or(Column::Index(1)),
        ),
    };
    let (source, target) = (resolve(&source)?, resolve(&target)?);
    let optional = |c: &Option<Column>| c.as_ref().map(resolve).transpose();
    let weight = optional(&options.weight)?;
    let time = optional(&options.time)?;
    let kind = optional(&options.kind)?;
    let extra = options
        .extra
        .iter()
        .map(resolve)
        .collect::<Result<_, _>>()?;
    let ids = match options.ids {
        IdType::Auto => guess_ids(rows, byte, source, target),
        given => given,
    };
    Ok(Found {
        layout: Layout {
            compression: Compression::None,
            delimiter: byte.map_or(Delimiter::Whitespace, Delimiter::Byte),
            header,
            source,
            target,
            weight,
            time,
            kind,
            extra,
            ids,
        },
        data_start: widen(data_start),
        lines_before,
    })
}

fn fields(line: &[u8], delimiter: Option<u8>) -> Vec<Field> {
    let mut out = Vec::new();
    // A malformed line still gives the fields before the fault.
    let _ = split_fields(line, delimiter, &mut out);
    out
}

fn field_texts(line: &[u8], delimiter: Option<u8>) -> Vec<String> {
    fields(line, delimiter)
        .into_iter()
        .map(|f| String::from_utf8_lossy(f.value(line)).replace("\"\"", "\""))
        .collect()
}

/// The first of tab, comma, semicolon and `|` found on the first line and the same
/// number of times (outside quotes) on more than half of the lines, else whitespace.
/// Not all of them: a malformed row must be reported, not change the delimiter.
fn guess_delimiter(content: &[(usize, &[u8])]) -> Delimiter {
    let count = |line: &[u8], d: u8| {
        let mut quoted = false;
        line.iter()
            .filter(|&&b| {
                quoted ^= b == b'"';
                b == d && !quoted
            })
            .count()
    };
    b"\t,;|"
        .iter()
        .find(|&&d| {
            let mut counts = content.iter().map(|(_, line)| count(line, d));
            counts.next().is_some_and(|first| {
                first > 0 && 2 * (1 + counts.filter(|&c| c == first).count()) > content.len()
            })
        })
        .map_or(Delimiter::Whitespace, |&d| Delimiter::Byte(d))
}

fn looks_like_header(content: &[(usize, &[u8])], delimiter: Option<u8>) -> bool {
    let first = content[0].1;
    let known = field_texts(first, delimiter)
        .iter()
        .any(|name| KNOWN.iter().any(|k| name.eq_ignore_ascii_case(k)));
    let digit = |line: &[u8]| line.iter().any(u8::is_ascii_digit);
    known || (!digit(first) && content.len() > 1 && content[1..].iter().all(|(_, l)| digit(l)))
}

fn resolve(column: &Column, header: Option<&[String]>) -> Result<usize, TextError> {
    match (column, header) {
        (Column::Index(index), Some(names)) if *index >= names.len() => {
            Err(TextError::BeyondHeader {
                index: *index,
                header: names.len(),
            })
        }
        (Column::Index(index), _) => Ok(*index),
        (Column::Name(name), None) => Err(TextError::NoHeader { name: name.clone() }),
        (Column::Name(name), Some(names)) => names
            .iter()
            .position(|n| n == name)
            .or_else(|| names.iter().position(|n| n.eq_ignore_ascii_case(name)))
            .ok_or_else(|| TextError::UnknownColumn {
                name: name.clone(),
                header: names.to_vec(),
            }),
    }
}

/// `from`/`to` or `source`/`target` in the header, else the first two columns.
fn default_pair(header: Option<&[String]>) -> (Column, Column) {
    let position = |name: &str| header?.iter().position(|n| n.eq_ignore_ascii_case(name));
    let (s, t) = [("from", "to"), ("source", "target")]
        .into_iter()
        .find_map(|(s, t)| Some((position(s)?, position(t)?)))
        .unwrap_or((0, 1));
    (Column::Index(s), Column::Index(t))
}

fn guess_ids(
    rows: &[(usize, &[u8])],
    delimiter: Option<u8>,
    source: usize,
    target: usize,
) -> IdType {
    let integers = rows.iter().all(|(_, line)| {
        let fields = fields(line, delimiter);
        [source, target]
            .into_iter()
            .all(|c| fields.get(c).is_none_or(|f| decimal(f.value(line)).is_ok()))
    });
    if integers {
        IdType::Integer
    } else {
        IdType::String
    }
}
