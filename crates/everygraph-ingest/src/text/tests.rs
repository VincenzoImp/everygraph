// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::io::Write;

use everygraph_core::MemorySource;
use ruzstd::encoding::{CompressionLevel, compress_to_vec};

use super::*;

/// One row, every column as comparable values.
type Row = (
    String,
    String,
    Option<u64>,
    Option<i64>,
    Option<Vec<u8>>,
    Vec<Vec<u8>>,
);

fn id_text(ids: &Ids, i: usize) -> String {
    match ids {
        Ids::Integer(v) => format!("#{}", v[i]),
        Ids::String(s) => String::from_utf8_lossy(s.get(i).unwrap()).into_owned(),
    }
}

fn flatten(batches: &[Batch]) -> Vec<Row> {
    let mut rows = Vec::new();
    for b in batches {
        assert!(b.weight.as_ref().is_none_or(|c| c.len() == b.len()));
        assert!(b.time.as_ref().is_none_or(|c| c.len() == b.len()));
        assert_eq!(b.target.len(), b.len());
        for i in 0..b.len() {
            rows.push((
                id_text(&b.source, i),
                id_text(&b.target, i),
                b.weight.as_ref().map(|w| w[i].to_bits()),
                b.time.as_ref().map(|t| t[i]),
                b.kind.as_ref().map(|k| k.get(i).unwrap().to_vec()),
                b.extra.iter().map(|e| e.get(i).unwrap().to_vec()).collect(),
            ));
        }
    }
    rows
}

fn skip_all(mut options: TextOptions) -> TextOptions {
    options.on_invalid = OnInvalid::Skip;
    options.max_problems = usize::MAX;
    options
}

type Read = Result<(Layout, Vec<Row>, Summary), String>;

/// The error is kept as its message, so that results can be compared.
fn read_with(bytes: &[u8], options: &TextOptions, tuning: Tuning) -> Read {
    let reader = TextReader::open_tuned(MemorySource::new(bytes), options, tuning)
        .map_err(|e| e.to_string())?;
    let out = reader.read(|b| b).map_err(|e| e.to_string())?;
    let layout = Layout {
        compression: Compression::None,
        ..reader.layout().clone()
    };
    Ok((layout, flatten(&out.batches), out.summary))
}

fn read(bytes: &[u8], options: &TextOptions) -> (Layout, Vec<Row>, Summary) {
    read_with(bytes, &skip_all(options.clone()), TUNING).unwrap()
}

fn int(a: u64, b: u64) -> Row {
    (
        format!("#{a}"),
        format!("#{b}"),
        None,
        None,
        None,
        Vec::new(),
    )
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// A zstd frame of raw blocks that declares its content size (`declared`), as libzstd
/// writes it; ruzstd's encoder does not declare it.
fn raw_frame(content: &[u8], declared: u64) -> Vec<u8> {
    // Single segment, 8-byte content size, no checksum, no dictionary.
    let mut out = [0x28, 0xB5, 0x2F, 0xFD, 0xE0].to_vec();
    out.extend_from_slice(&declared.to_le_bytes());
    let blocks: Vec<&[u8]> = if content.is_empty() {
        vec![&[]]
    } else {
        content.chunks(1000).collect()
    };
    for (i, block) in blocks.iter().enumerate() {
        let last = u32::from(i + 1 == blocks.len());
        let header = (u32::try_from(block.len()).unwrap() << 3) | last;
        out.extend_from_slice(&header.to_le_bytes()[..3]);
        out.extend_from_slice(block);
    }
    out
}

/// zstd frames of `bytes` cut every `every` bytes, with a skippable frame among them;
/// frames alternate between declared sizes (decoded in parallel) and undeclared ones
/// (streamed).
fn zstd_frames(bytes: &[u8], every: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, part) in bytes.chunks(every.max(1)).enumerate() {
        if i == 1 {
            out.extend_from_slice(&0x184D_2A53_u32.to_le_bytes());
            out.extend_from_slice(&3_u32.to_le_bytes());
            out.extend_from_slice(b"abc");
        }
        if i % 2 == 0 {
            out.extend_from_slice(&raw_frame(part, widen(part.len())));
        } else {
            out.extend_from_slice(&compress_to_vec(part, CompressionLevel::Fastest));
        }
    }
    if bytes.is_empty() {
        out = compress_to_vec(bytes, CompressionLevel::Fastest);
    }
    out
}

/// Tunings small enough that every boundary falls inside lines, fields and line ends.
fn tunings() -> Vec<Tuning> {
    let mut all = Vec::new();
    for (chunk, step) in [
        (1, 1),
        (2, 1),
        (3, 2),
        (5, 3),
        (7, 8),
        (16, 4),
        (64, 64),
        (4096, 4096),
    ] {
        all.push(Tuning {
            chunk,
            step,
            window: chunk * 3,
            piece: chunk.div_ceil(2),
            max_line: 80,
            zstd_group: widen(chunk) * 8,
        });
    }
    all
}

/// Every source of the same text, at every tuning, gives the same layout, rows and
/// problems.
fn assert_same_everywhere(text: &[u8], options: &TextOptions) {
    let options = skip_all(options.clone());
    let whole = text.len() + 1;
    let one_chunk = Tuning {
        chunk: whole,
        window: whole,
        piece: whole,
        ..tunings()[0]
    };
    let expected = read_with(text, &options, one_chunk);
    let encoded: Vec<(&str, Vec<u8>)> = vec![
        ("plain", text.to_vec()),
        ("gzip", gzip(text)),
        (
            "gzip, two members",
            [gzip(&text[..text.len() / 2]), gzip(&text[text.len() / 2..])].concat(),
        ),
        ("zstd", zstd_frames(text, usize::MAX)),
        ("zstd, frames", zstd_frames(text, 11)),
    ];
    for (name, bytes) in &encoded {
        for tuning in tunings() {
            let context = format!(
                "{name}, {tuning:?}, text {:?}",
                String::from_utf8_lossy(text)
            );
            assert_eq!(read_with(bytes, &options, tuning), expected, "{context}");
        }
    }
}

#[test]
fn snap_file_with_comments_crlf_and_tabs() {
    let text = b"# Directed graph\r\n# FromNodeId\tToNodeId\r\n0\t1\r\n0\t2\r\n\r\n7\t18446744073709551615\r\n";
    let (layout, rows, summary) = read(text, &TextOptions::default());
    assert_eq!(layout.delimiter, Delimiter::Byte(b'\t'));
    assert_eq!(layout.header, None);
    assert_eq!(layout.ids, IdType::Integer);
    assert_eq!(rows, [int(0, 1), int(0, 2), int(7, u64::MAX)]);
    assert_eq!(
        (summary.lines, summary.rows, summary.invalid_rows),
        (6, 3, 0)
    );
    assert_same_everywhere(text, &TextOptions::default());
}

#[test]
fn space_separated_without_final_line_end() {
    let text = b"1 2\n  3   4  \n5 6";
    let (layout, rows, summary) = read(text, &TextOptions::default());
    assert_eq!(layout.delimiter, Delimiter::Whitespace);
    assert_eq!(rows, [int(1, 2), int(3, 4), int(5, 6)]);
    assert_eq!(summary.lines, 3);
    assert_same_everywhere(text, &TextOptions::default());
}

#[test]
fn csv_with_header_quotes_weight_time_and_extras() {
    let text = "\u{feff}from,to,value,ts,kind,note\n\
        0xAB,\"c,d\",1.5,2024-03-02T11:04:35Z,transfer,\"say \"\"hi\"\"\"\n \
        e , f ,-2e3,1700000000,  ,\n"
        .as_bytes();
    let options = TextOptions {
        weight: Some("value".into()),
        time: Some(Column::Name("TS".into())),
        kind: Some(4.into()),
        extra: vec!["note".into()],
        ..TextOptions::default()
    };
    let (layout, rows, summary) = read(text, &options);
    assert_eq!(layout.delimiter, Delimiter::Byte(b','));
    assert_eq!(layout.header.as_ref().unwrap()[0], "from");
    assert_eq!((layout.source, layout.target, layout.time), (0, 1, Some(3)));
    assert_eq!(layout.ids, IdType::String);
    let row = |s: &str, t: &str, w: f64, time: i64, kind: &str, note: &str| {
        (
            s.to_owned(),
            t.to_owned(),
            Some(w.to_bits()),
            Some(time),
            Some(kind.as_bytes().to_vec()),
            vec![note.as_bytes().to_vec()],
        )
    };
    assert_eq!(
        rows,
        [
            row(
                "0xAB",
                "c,d",
                1.5,
                1_709_377_475_000_000,
                "transfer",
                "say \"hi\""
            ),
            row("e", "f", -2000.0, 1_700_000_000_000_000, "", ""),
        ]
    );
    assert_eq!((summary.lines, summary.rows), (3, 2));
    assert_same_everywhere(text, &options);
}

#[test]
fn source_and_target_default_to_named_columns() {
    let (layout, rows, _) = read(b"weight;target;source\n1;2;3\n", &TextOptions::default());
    assert_eq!(layout.delimiter, Delimiter::Byte(b';'));
    assert_eq!((layout.source, layout.target), (2, 1));
    assert_eq!(rows, [int(3, 2)]);
}

#[test]
fn header_detection() {
    let header = |text: &[u8]| read(text, &TextOptions::default()).0.header;
    let names = |n: &[&str]| Some(n.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    // A known name.
    assert_eq!(header(b"src dst\n1 2\n"), names(&["src", "dst"]));
    assert_eq!(header(b"From,To\n"), names(&["From", "To"]));
    // No digit on the first line, one on every other.
    assert_eq!(header(b"a|b\n0xa1|0xb2\n"), names(&["a", "b"]));
    // Names all through: no header.
    assert_eq!(header(b"alice,bob\ncarol,dave\n"), None);
    assert_eq!(header(b"1 2\n"), None);
    // Forced either way, and implied by a column name.
    let present = TextOptions {
        header: Header::Present,
        ..TextOptions::default()
    };
    assert_eq!(read(b"x y\n1 2\n", &present).0.header, names(&["x", "y"]));
    let absent = TextOptions {
        header: Header::Absent,
        ..TextOptions::default()
    };
    assert_eq!(
        read(b"from to\n1 2\n", &absent).1,
        [
            ("from".into(), "to".into(), None, None, None, vec![]),
            ("1".into(), "2".into(), None, None, None, vec![])
        ]
    );
    let named = TextOptions {
        source: Some("y".into()),
        target: Some("x".into()),
        ..TextOptions::default()
    };
    assert_eq!(read(b"x y\n1 2\n", &named).1, [int(2, 1)]);
}

#[test]
fn empty_and_header_only_inputs_have_no_rows() {
    for text in [&b""[..], b"\n\n", b"# only a comment\n", b"\xEF\xBB\xBF"] {
        let (_, rows, summary) = read(text, &TextOptions::default());
        assert!(rows.is_empty(), "{text:?}");
        assert_eq!(summary.invalid_rows, 0);
        assert_same_everywhere(text, &TextOptions::default());
    }
    let (layout, rows, summary) = read(b"from,to\r\n", &TextOptions::default());
    assert_eq!(layout.header.unwrap().len(), 2);
    assert!(rows.is_empty());
    assert_eq!(summary.lines, 1);
    let (layout, rows, _) = read(b"source\ttarget", &TextOptions::default());
    assert!(layout.header.is_some() && rows.is_empty());
}

#[test]
fn integer_ids_reject_overflow_instead_of_wrapping() {
    let text = b"18446744073709551615 0\n18446744073709551616 0\n99999999999999999999x 0\n-1 0\n";
    let options = TextOptions {
        ids: IdType::Integer,
        ..TextOptions::default()
    };
    let (_, rows, summary) = read(text, &options);
    assert_eq!(rows, [int(u64::MAX, 0)]);
    let kinds: Vec<_> = summary.problems.iter().map(|p| (p.line, p.kind)).collect();
    assert_eq!(
        kinds,
        [
            (2, ProblemKind::TooLarge),
            (3, ProblemKind::NotAnInteger),
            (4, ProblemKind::NotAnInteger)
        ]
    );
    assert_eq!(
        summary.problems[0].to_string(),
        "column 1 is larger than 18446744073709551615"
    );
    assert_eq!(summary.problems[0].span, Some(0..20));
}

#[test]
fn auto_ids_fall_back_to_strings() {
    let (layout, rows, _) = read(b"1 2\n3 x\n", &TextOptions::default());
    assert_eq!(layout.ids, IdType::String);
    assert_eq!(rows[1].1, "x");
    // An id too large for 64 bits is a string to detection, not an error.
    assert_eq!(
        read(b"1 99999999999999999999\n", &TextOptions::default())
            .0
            .ids,
        IdType::String
    );
}

#[test]
fn hex_ids() {
    let options = TextOptions {
        ids: IdType::Hex,
        ..TextOptions::default()
    };
    let (_, rows, summary) = read(
        b"0x1f FF\n0Xffffffffffffffff 0\n0x 1\n0x10000000000000000 1\nzz 1\n",
        &options,
    );
    assert_eq!(rows, [int(31, 255), int(u64::MAX, 0)]);
    let kinds: Vec<_> = summary.problems.iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        [
            ProblemKind::NotHex,
            ProblemKind::TooLarge,
            ProblemKind::NotHex
        ]
    );
}

#[test]
fn refuse_reads_everything_then_refuses() {
    let text = b"a,b\n1,2\n3\n4,5,6\n7,8\n";
    let reader = TextReader::open(MemorySource::new(&text[..]), &TextOptions::default()).unwrap();
    let Err(TextError::InvalidRows(summary)) = reader.read(|b| b.len()) else {
        panic!("expected InvalidRows");
    };
    assert_eq!(
        (summary.lines, summary.rows, summary.invalid_rows),
        (5, 2, 2)
    );
    assert_eq!(
        summary.problems[0].to_string(),
        "1 field where the header has 2"
    );
    assert_eq!(
        summary.problems[1].to_string(),
        "3 fields where the header has 2"
    );
    assert_eq!(
        TextError::InvalidRows(summary).to_string(),
        "2 lines are not valid rows"
    );
}

#[test]
fn problems_are_capped_but_all_counted() {
    let text = b"x\n".repeat(50);
    let options = TextOptions {
        on_invalid: OnInvalid::Skip,
        max_problems: 3,
        ..TextOptions::default()
    };
    let tuning = Tuning {
        chunk: 7,
        ..tunings()[3]
    };
    let reader =
        TextReader::open_tuned(MemorySource::new(text.as_slice()), &options, tuning).unwrap();
    let summary = reader.read(|_| ()).unwrap().summary;
    assert_eq!(summary.invalid_rows, 50);
    assert_eq!(
        summary.problems.iter().map(|p| p.line).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        summary.problems[0].to_string(),
        "1 field where 2 are needed"
    );
}

#[test]
fn columns_that_do_not_exist() {
    let open = |text: &'static [u8], options: TextOptions| {
        TextReader::open(MemorySource::new(text), &options).map(|_| ())
    };
    let named = |n: &str| TextOptions {
        weight: Some(n.into()),
        ..TextOptions::default()
    };
    assert!(matches!(
        open(b"from,to\n", named("value")),
        Err(TextError::UnknownColumn { .. })
    ));
    assert!(matches!(
        open(
            b"1,2\n",
            TextOptions {
                header: Header::Absent,
                ..named("value")
            }
        ),
        Err(TextError::NoHeader { .. })
    ));
    let beyond = TextOptions {
        weight: Some(2.into()),
        ..TextOptions::default()
    };
    let error = open(b"from,to\n", beyond).unwrap_err();
    assert_eq!(
        error.to_string(),
        "column 3 was asked for and the header has 2"
    );
}

#[test]
fn lines_too_long_are_invalid_rows_and_do_not_shift_line_numbers() {
    let mut text = b"1 2\n".to_vec();
    text.extend(std::iter::repeat_n(b'9', 100));
    text.extend_from_slice(b" 1\n3 4\n");
    text.extend(std::iter::repeat_n(b'8', 100));
    let (_, _, summary) =
        read_with(&text, &skip_all(TextOptions::default()), tunings()[2]).unwrap();
    let kinds: Vec<_> = summary.problems.iter().map(|p| (p.line, p.kind)).collect();
    assert_eq!(
        kinds,
        [
            (2, ProblemKind::LineTooLong { limit: 80 }),
            (4, ProblemKind::LineTooLong { limit: 80 })
        ]
    );
    assert_eq!(summary.rows, 2);
    assert_same_everywhere(&text, &TextOptions::default());
}

#[test]
fn every_source_and_tuning_agree_on_generated_inputs() {
    const TOKENS: &[&str] = &[
        "1",
        "22",
        "x",
        " ",
        "\t",
        ",",
        "\"",
        "\"\"",
        "\r",
        "\n",
        "\n",
        "\n",
        "#",
        "-",
        ".5",
        "2024-01-02",
        "18446744073709551616",
        "aaaaaaaaaaaaaaaaaaaa",
    ];
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let csv = TextOptions {
        delimiter: Delimiter::Byte(b','),
        header: Header::Absent,
        weight: Some(2.into()),
        ..TextOptions::default()
    };
    let ws = TextOptions {
        delimiter: Delimiter::Whitespace,
        ids: IdType::Integer,
        time: Some(2.into()),
        ..TextOptions::default()
    };
    let strings = TextOptions {
        delimiter: Delimiter::Byte(b'\t'),
        ids: IdType::String,
        extra: vec![2.into()],
        ..TextOptions::default()
    };
    for case in 0..120 {
        let len = usize::try_from(next() % 60).unwrap();
        let text: String = (0..len)
            .map(|_| TOKENS[usize::try_from(next() % widen(TOKENS.len())).unwrap()])
            .collect();
        let options = [&csv, &ws, &strings][case % 3];
        assert_same_everywhere(text.as_bytes(), options);
    }
}

#[test]
fn arbitrary_bytes_do_not_panic() {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    for _ in 0..200 {
        let bytes: Vec<u8> = (0..64)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.to_le_bytes()[0]
            })
            .collect();
        for tuning in tunings() {
            if let Ok(reader) = TextReader::open_tuned(
                MemorySource::new(bytes.as_slice()),
                &skip_all(TextOptions::default()),
                tuning,
            ) {
                let _ = reader.read(|b| b.len());
            }
        }
    }
}

#[test]
fn damaged_compressed_data_is_an_error() {
    let text = b"1 2\n3 4\n".repeat(100);
    for bytes in [gzip(&text), zstd_frames(&text, 300)] {
        let truncated = &bytes[..bytes.len() - 5];
        let result = TextReader::open(MemorySource::new(truncated), &TextOptions::default())
            .and_then(|r| r.read(|_| ()));
        assert!(
            matches!(result, Err(TextError::Damaged { .. })),
            "{result:?}"
        );
    }
    let mut corrupt = compress_to_vec(text.as_slice(), CompressionLevel::Fastest);
    let n = corrupt.len();
    corrupt[n - 2] ^= 0xff; // in the checksum
    let result = TextReader::open(
        MemorySource::new(corrupt.as_slice()),
        &TextOptions::default(),
    )
    .and_then(|r| r.read(|_| ()));
    assert!(
        matches!(result, Err(TextError::Damaged { .. })),
        "{result:?}"
    );
}

#[test]
fn a_zstd_frame_that_expands_past_its_cap_is_streamed() {
    // Two RLE blocks, 4 bytes each, that decode to 100,000 line ends each.
    let mut bomb = [0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x50].to_vec();
    for last in [0, 1] {
        bomb.extend_from_slice(&((100_000_u32 << 3) | (1 << 1) | last).to_le_bytes()[..3]);
        bomb.push(b'\n');
    }
    let text = b"1 2\n3 4\n";
    let bytes = [raw_frame(text, 8), bomb, raw_frame(text, 8)].concat();
    let (_, rows, summary) = read(&bytes, &TextOptions::default());
    assert_eq!(rows, [int(1, 2), int(3, 4), int(1, 2), int(3, 4)]);
    assert_eq!(summary.lines, 200_004);
}

#[test]
fn a_second_pass_gives_the_same_batches() {
    let text = b"1 2\n3 4\n5 6\n".repeat(1000);
    let reader = TextReader::open_tuned(
        MemorySource::new(text.as_slice()),
        &TextOptions::default(),
        Tuning {
            chunk: 4096,
            ..TUNING
        },
    )
    .unwrap();
    let first = reader.read(|b| b).unwrap();
    let second = reader.read(|b| b).unwrap();
    assert!(first.batches.len() > 1);
    assert_eq!(first.batches, second.batches);
    assert_eq!(first.summary, second.summary);
}
