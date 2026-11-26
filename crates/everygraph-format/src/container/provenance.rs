// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Provenance records (`docs/spec/eg-container.md` § 6): how a section was made.

use std::collections::BTreeMap;

use super::{ContainerError, MAX_PROVENANCE_BYTES};

const MAGIC: [u8; 8] = *b"EGPROV\0\0";
const VERSION: u32 = 1;
/// An unknown time is written as this value.
const UNKNOWN_TIME: i64 = i64::MIN;

/// How one or more sections were made: what went in, with which parameters, by which
/// engine, when. Stored as a section of kind [`Kind::PROVENANCE`](super::Kind::PROVENANCE)
/// that other sections refer to by name. The crate does not read the clock or the
/// file system: the caller fills every field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provenance {
    /// The program and version that wrote the sections, such as `"everygraph 0.1.0"`.
    pub engine: String,
    /// What was run, such as `"ingest"` or `"pagerank"`.
    pub operation: String,
    /// When it started, in nanoseconds since 1970-01-01 UTC; `None` if not known.
    pub started: Option<i64>,
    /// When it finished, in the same unit.
    pub finished: Option<i64>,
    /// The inputs it read.
    pub inputs: Vec<Input>,
    /// Every parameter that can change what was written, as text.
    pub parameters: BTreeMap<String, String>,
}

/// The fingerprint of one input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Input {
    /// How the input was named (a path as given, a URL, a label).
    pub name: String,
    /// Its length in bytes.
    pub bytes: u64,
    /// Its modification time, in nanoseconds since 1970-01-01 UTC; `None` if not known.
    pub modified: Option<i64>,
    /// What [`digest`](Self::digest) is, such as `"blake3"` (of the whole input) or
    /// `"blake3-first-last-1MiB"` (`docs/ux-contract.md` R-6); empty if there is none.
    pub algorithm: String,
    /// The digest's bytes.
    pub digest: Vec<u8>,
}

fn time(t: Option<i64>) -> Result<i64, ContainerError> {
    match t {
        None => Ok(UNKNOWN_TIME),
        Some(UNKNOWN_TIME) => Err(ContainerError::InvalidProvenance(
            "i64::MIN is the code for an unknown time",
        )),
        Some(t) => Ok(t),
    }
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ContainerError> {
    let len = u32::try_from(bytes.len())
        .map_err(|_| ContainerError::TooLarge("a provenance string exceeds 4 GiB"))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn put_count(out: &mut Vec<u8>, n: usize) -> Result<(), ContainerError> {
    let n = u32::try_from(n).map_err(|_| ContainerError::TooLarge("provenance count"))?;
    out.extend_from_slice(&n.to_le_bytes());
    Ok(())
}

impl Provenance {
    /// The record's bytes.
    pub(crate) fn encode(&self) -> Result<Vec<u8>, ContainerError> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&time(self.started)?.to_le_bytes());
        out.extend_from_slice(&time(self.finished)?.to_le_bytes());
        put_bytes(&mut out, self.engine.as_bytes())?;
        put_bytes(&mut out, self.operation.as_bytes())?;
        put_count(&mut out, self.inputs.len())?;
        for input in &self.inputs {
            put_bytes(&mut out, input.name.as_bytes())?;
            out.extend_from_slice(&input.bytes.to_le_bytes());
            out.extend_from_slice(&time(input.modified)?.to_le_bytes());
            put_bytes(&mut out, input.algorithm.as_bytes())?;
            put_bytes(&mut out, &input.digest)?;
        }
        put_count(&mut out, self.parameters.len())?;
        for (key, value) in &self.parameters {
            put_bytes(&mut out, key.as_bytes())?;
            put_bytes(&mut out, value.as_bytes())?;
        }
        if out.len() as u64 > MAX_PROVENANCE_BYTES {
            return Err(ContainerError::TooLarge(
                "a provenance record exceeds 16 MiB",
            ));
        }
        Ok(out)
    }

    /// Decodes a record's bytes.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, ContainerError> {
        let mut r = Reader { bytes, at: 0 };
        if r.take(8)? != MAGIC {
            return Err(bad("no magic"));
        }
        if r.u32()? != VERSION {
            return Err(bad("unknown record version"));
        }
        r.take(4)?;
        let started = opt_time(r.i64()?);
        let finished = opt_time(r.i64()?);
        let engine = r.text()?;
        let operation = r.text()?;
        let mut inputs = Vec::new();
        for _ in 0..r.u32()? {
            inputs.push(Input {
                name: r.text()?,
                bytes: r.u64()?,
                modified: opt_time(r.i64()?),
                algorithm: r.text()?,
                digest: r.bytes()?.to_vec(),
            });
        }
        let mut parameters = BTreeMap::new();
        let mut last: Option<String> = None;
        for _ in 0..r.u32()? {
            let key = r.text()?;
            if last.as_ref().is_some_and(|l| *l >= key) {
                return Err(bad("parameter names are not unique and increasing"));
            }
            last = Some(key.clone());
            parameters.insert(key, r.text()?);
        }
        if r.at != bytes.len() {
            return Err(bad("bytes after the end of the record"));
        }
        Ok(Self {
            engine,
            operation,
            started,
            finished,
            inputs,
            parameters,
        })
    }
}

const fn opt_time(t: i64) -> Option<i64> {
    if t == UNKNOWN_TIME { None } else { Some(t) }
}

const fn bad(what: &'static str) -> ContainerError {
    ContainerError::InvalidProvenance(what)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ContainerError> {
        let end = self.at.checked_add(n).ok_or(bad("truncated"))?;
        let out = self.bytes.get(self.at..end).ok_or(bad("truncated"))?;
        self.at = end;
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32, ContainerError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }

    fn u64(&mut self) -> Result<u64, ContainerError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }

    fn i64(&mut self) -> Result<i64, ContainerError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }

    fn bytes(&mut self) -> Result<&'a [u8], ContainerError> {
        let n = usize::try_from(self.u32()?).map_err(|_| bad("truncated"))?;
        self.take(n)
    }

    fn text(&mut self) -> Result<String, ContainerError> {
        let b = self.bytes()?;
        Ok(std::str::from_utf8(b)
            .map_err(|_| bad("text is not UTF-8"))?
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Provenance {
        Provenance {
            engine: "everygraph 0.0.0".into(),
            operation: "ingest".into(),
            started: Some(1_764_000_000_000_000_000),
            finished: None,
            inputs: vec![Input {
                name: "transfers.parquet".into(),
                bytes: 38_200_000_000,
                modified: Some(-5),
                algorithm: "blake3-first-last-1MiB".into(),
                digest: vec![1, 2, 3],
            }],
            parameters: [("source", "from"), ("target", "to")]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        }
    }

    #[test]
    fn a_record_round_trips() {
        let p = sample();
        assert_eq!(Provenance::decode(&p.encode().unwrap()).unwrap(), p);
        let empty = Provenance::default();
        assert_eq!(Provenance::decode(&empty.encode().unwrap()).unwrap(), empty);
    }

    #[test]
    fn a_damaged_record_is_refused_not_misread() {
        let bytes = sample().encode().unwrap();
        for cut in 0..bytes.len() {
            assert!(Provenance::decode(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(Provenance::decode(&longer).is_err());
        assert!(
            Provenance {
                started: Some(i64::MIN),
                ..Provenance::default()
            }
            .encode()
            .is_err()
        );
    }
}
