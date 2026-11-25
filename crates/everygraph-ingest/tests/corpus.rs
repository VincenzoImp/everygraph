// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The malformed-input corpus of the text reader (`tests/corpus/`): every invalid row is
//! reported at the line and with the sentence that `tests/corpus/index.txt` gives
//! (`docs/ux-contract.md` § 6.1, M-6), and gzip and zstd copies of each file give the
//! same rows and problems as the file.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use everygraph_core::MemorySource;
use everygraph_ingest::text::{
    Batch, Column, Header, IdType, OnInvalid, TextError, TextOptions, TextReader, TimeUnit,
};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

/// One file of the index: its options and its problems, as `FILE:LINE  MESSAGE`.
struct Entry {
    options: TextOptions,
    problems: Vec<String>,
}

fn column(value: &str) -> Column {
    value
        .parse()
        .map_or_else(|_| Column::Name(value.to_owned()), Column::Index)
}

/// The keys of an `@` line.
fn options(keys: &[&str]) -> TextOptions {
    let mut options = TextOptions {
        on_invalid: OnInvalid::Skip,
        max_problems: usize::MAX,
        ..TextOptions::default()
    };
    for key in keys {
        let (key, value) = key.split_once('=').expect("KEY=VALUE");
        match key {
            "source" => options.source = Some(column(value)),
            "target" => options.target = Some(column(value)),
            "weight" => options.weight = Some(column(value)),
            "time" => options.time = Some(column(value)),
            "kind" => options.kind = Some(column(value)),
            "extra" => options.extra.push(column(value)),
            "header" => {
                options.header = if value == "yes" {
                    Header::Present
                } else {
                    Header::Absent
                }
            }
            "ids" => {
                options.ids = match value {
                    "integer" => IdType::Integer,
                    "hex" => IdType::Hex,
                    _ => IdType::String,
                };
            }
            "unit" => {
                options.time_unit = match value {
                    "ms" => TimeUnit::Milliseconds,
                    "us" => TimeUnit::Microseconds,
                    _ => TimeUnit::Seconds,
                };
            }
            other => panic!("unknown key {other} in the index"),
        }
    }
    options
}

fn index() -> BTreeMap<String, Entry> {
    let text = std::fs::read_to_string(corpus().join("index.txt")).unwrap();
    let mut entries = BTreeMap::new();
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if let Some(rest) = line.strip_prefix("@ ") {
            let mut words = rest.split_whitespace();
            let file = words.next().unwrap().to_owned();
            let keys: Vec<&str> = words.collect();
            entries.insert(
                file,
                Entry {
                    options: options(&keys),
                    problems: Vec::new(),
                },
            );
        } else {
            let (file, _) = line.split_once(':').unwrap();
            entries
                .get_mut(file)
                .expect("an @ line comes first")
                .problems
                .push(line.to_owned());
        }
    }
    entries
}

fn read(
    bytes: &[u8],
    options: &TextOptions,
) -> Result<(Vec<Batch>, everygraph_ingest::text::Summary), TextError> {
    let reader = TextReader::open(MemorySource::new(bytes), options)?;
    let out = reader.read(|batch| batch)?;
    Ok((out.batches, out.summary))
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn zstd(bytes: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest)
}

#[test]
fn every_invalid_row_is_reported_where_the_index_says() {
    let index = index();
    for (file, entry) in &index {
        let bytes = std::fs::read(corpus().join(file)).unwrap();
        let (batches, summary) = read(&bytes, &entry.options).unwrap();
        let found: Vec<String> = summary
            .problems
            .iter()
            .map(|p| format!("{file}:{}  {p}", p.line))
            .collect();
        assert_eq!(found, entry.problems, "{file}");
        assert_eq!(
            summary.invalid_rows,
            u64::try_from(entry.problems.len()).unwrap(),
            "{file}"
        );
        let rows: usize = batches.iter().map(Batch::len).sum();
        assert_eq!(summary.rows, u64::try_from(rows).unwrap(), "{file}");

        // The location points at the line the excerpt came from.
        let lines: Vec<&[u8]> = bytes.split(|&b| b == b'\n').collect();
        for problem in &summary.problems {
            let line = lines[usize::try_from(problem.line).unwrap() - 1];
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            assert_eq!(
                problem.excerpt,
                &line[..line.len().min(160)],
                "{file}:{}",
                problem.line
            );
        }

        // Refused by default, with the same count.
        let refuse = TextOptions {
            on_invalid: OnInvalid::Refuse,
            ..entry.options.clone()
        };
        match read(&bytes, &refuse) {
            Ok(_) => assert!(entry.problems.is_empty(), "{file}"),
            Err(TextError::InvalidRows(refused)) => {
                assert_eq!(refused.invalid_rows, summary.invalid_rows, "{file}");
                assert_eq!(refused.lines, summary.lines, "{file}");
            }
            Err(other) => panic!("{file}: {other}"),
        }

        // Compressed copies: made here, and the ones made by the gzip and zstd tools.
        let mut copies = vec![("gzip", gzip(&bytes)), ("zstd", zstd(&bytes))];
        for extension in ["gz", "zst"] {
            if let Ok(copy) = std::fs::read(corpus().join(format!("{file}.{extension}"))) {
                copies.push((extension, copy));
            }
        }
        for (name, copy) in copies {
            let (copy_batches, copy_summary) = read(&copy, &entry.options).unwrap();
            assert_eq!(copy_summary, summary, "{file}, {name}");
            assert_eq!(copy_batches, batches, "{file}, {name}");
        }
    }

    // Every file of the corpus is in the index.
    for entry in std::fs::read_dir(corpus()).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let plain = name.trim_end_matches(".gz").trim_end_matches(".zst");
        assert!(
            name == "index.txt" || name == ".gitattributes" || index.contains_key(plain),
            "{name} is not in the index"
        );
    }
}
