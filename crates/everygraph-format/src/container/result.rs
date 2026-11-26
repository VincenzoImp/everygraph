// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Results kept in the graph file (`docs/spec/eg-container.md` § 6, "Results"): the
//! graph identity, the key a result is found by, and result sections of typed values.

use std::collections::BTreeMap;
use std::fmt;

use everygraph_core::{ByteSink, ByteSource};

use super::read::{Container, Section};
use super::write::Writer;
use super::{ContainerError, Entry, Kind, Provenance};

const MAGIC: [u8; 8] = *b"EGRES\0\0\0";
/// Magic, value type, reserved: the values start 16 bytes into the section.
const HEAD: usize = 16;
/// Bytes encoded or decoded at a time.
const CHUNK: usize = 1 << 14;
/// The parameters every result key holds (spec § 6).
const RESERVED: [&str; 3] = ["deterministic", "graph", "version"];

/// What a result was computed on: BLAKE3 over the sections that define the graph
/// (spec § 6). Two files whose graph sections differ in kind, name or content never
/// share one; results, provenance and anything else do not enter it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GraphIdentity([u8; 32]);

impl GraphIdentity {
    /// The identity of the graph whose sections are among `entries`, which are in
    /// increasing byte order of name.
    fn of<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Self {
        let mut hasher = blake3::Hasher::new();
        for entry in entries.into_iter().filter(|e| e.kind.defines_graph()) {
            // Names are at most 1024 bytes (spec § 4).
            let name_len = u32::try_from(entry.name.len()).unwrap_or(u32::MAX);
            hasher.update(&entry.kind.code().to_le_bytes());
            hasher.update(&name_len.to_le_bytes());
            hasher.update(entry.name.as_bytes());
            hasher.update(&entry.hash);
        }
        Self(*hasher.finalize().as_bytes())
    }

    /// The 32 bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// 64 lowercase hexadecimal digits, as a provenance record writes it.
impl fmt::Display for GraphIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for GraphIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GraphIdentity({self})")
    }
}

/// What a stored result is found by: the graph it was computed on, the operation and
/// its version, whether the run was deterministic, and every parameter that can change
/// the result (`docs/api-sketch.md` § 2.8). The memory budget, the thread count and
/// anything else that cannot change the result are not part of it.
///
/// A result of a newer version of the operation, or a `fast` run, is a different key,
/// so it is never served for a request it does not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultKey {
    graph: GraphIdentity,
    operation: String,
    parameters: BTreeMap<String, String>,
}

impl ResultKey {
    /// The key of `operation` (such as `"pagerank"`) at algorithm version `version`,
    /// run deterministically or not, on the graph `graph`. Add the parameters with
    /// [`with`](Self::with).
    #[must_use]
    pub fn new(graph: GraphIdentity, operation: &str, version: u32, deterministic: bool) -> Self {
        let parameters = [
            ("deterministic", deterministic.to_string()),
            ("graph", graph.to_string()),
            ("version", version.to_string()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        Self {
            graph,
            operation: operation.to_owned(),
            parameters,
        }
    }

    /// Adds parameter `name` with `value`, written as spec § 6 says (counts and seeds
    /// in decimal, floating-point values as Rust's `{}` writes them).
    ///
    /// # Errors
    ///
    /// [`ContainerError::InvalidResult`] for `deterministic`, `graph` and `version`,
    /// which [`new`](Self::new) sets.
    pub fn with(mut self, name: &str, value: &str) -> Result<Self, ContainerError> {
        if RESERVED.contains(&name) {
            return Err(ContainerError::InvalidResult(
                "deterministic, graph and version are set by ResultKey::new",
            ));
        }
        self.parameters.insert(name.to_owned(), value.to_owned());
        Ok(self)
    }

    /// The graph the result is of.
    #[must_use]
    pub const fn graph(&self) -> GraphIdentity {
        self.graph
    }

    /// The operation.
    #[must_use]
    pub fn operation(&self) -> &str {
        &self.operation
    }

    /// Every parameter, those [`new`](Self::new) sets included, as the provenance
    /// record holds them.
    #[must_use]
    pub const fn parameters(&self) -> &BTreeMap<String, String> {
        &self.parameters
    }

    /// `results/<operation>/<32 hexadecimal digits of the key's hash>`.
    fn section_name(&self) -> Result<String, ContainerError> {
        let key = Provenance {
            operation: self.operation.clone(),
            parameters: self.parameters.clone(),
            ..Provenance::default()
        }
        .encode()?;
        let hash = blake3::hash(&key);
        let digits: String = hash.as_bytes()[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Ok(format!("results/{}/{digits}", self.operation))
    }

    fn matches(&self, record: &Provenance) -> bool {
        record.operation == self.operation && record.parameters == self.parameters
    }
}

/// The type of the values of a result section (spec § 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ValueType {
    /// `u32`, such as a component or community label.
    U32,
    /// `u64`, such as a count.
    U64,
    /// IEEE 754 binary32.
    F32,
    /// IEEE 754 binary64, such as a `PageRank` score.
    F64,
}

impl ValueType {
    const fn code(self) -> u32 {
        match self {
            Self::U32 => 1,
            Self::U64 => 2,
            Self::F32 => 3,
            Self::F64 => 4,
        }
    }

    const fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(Self::U32),
            2 => Some(Self::U64),
            3 => Some(Self::F32),
            4 => Some(Self::F64),
            _ => None,
        }
    }

    /// Bytes per value.
    #[must_use]
    pub const fn size(self) -> usize {
        match self {
            Self::U32 | Self::F32 => 4,
            Self::U64 | Self::F64 => 8,
        }
    }
}

/// A value a result section holds: `u32`, `u64`, `f32` or `f64`, little-endian.
pub trait Value: Copy {
    /// Its type in the section.
    const TYPE: ValueType;
    /// Writes the value into `out`, [`ValueType::size`] bytes. Floating-point values
    /// are written canonically (spec § 1): NaN as the quiet NaN, −0.0 as +0.0.
    fn to_le(self, out: &mut [u8]);
    /// Reads a value from [`ValueType::size`] bytes.
    fn from_le(bytes: &[u8]) -> Self;
}

macro_rules! value {
    ($t:ty, $type:ident, $canonical:expr) => {
        impl Value for $t {
            const TYPE: ValueType = ValueType::$type;
            fn to_le(self, out: &mut [u8]) {
                let canonical: fn($t) -> $t = $canonical;
                out.copy_from_slice(&canonical(self).to_le_bytes());
            }
            fn from_le(bytes: &[u8]) -> Self {
                <$t>::from_le_bytes(bytes.try_into().expect("one value"))
            }
        }
    };
}

value!(u32, U32, |v| v);
value!(u64, U64, |v| v);
// `-0.0 == 0.0`, so both zeros are written as +0.0.
value!(f32, F32, |v| if v.is_nan() {
    f32::NAN
} else if v == 0.0 {
    0.0
} else {
    v
});
value!(f64, F64, |v| if v.is_nan() {
    f64::NAN
} else if v == 0.0 {
    0.0
} else {
    v
});

/// A result found in a graph file: see [`Container::find_result`].
#[derive(Debug)]
pub struct StoredResult<'a, S> {
    section: Section<'a, S>,
    value_type: ValueType,
    len: u64,
    provenance: Provenance,
}

impl<S: ByteSource> StoredResult<'_, S> {
    /// The type of the values.
    #[must_use]
    pub const fn value_type(&self) -> ValueType {
        self.value_type
    }

    /// The number of values.
    #[must_use]
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// Whether there are none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How the result was made: its key, the engine, the times.
    #[must_use]
    pub const fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    /// Fills `out` with the values from number `first` on.
    ///
    /// # Errors
    ///
    /// [`ContainerError::InvalidResult`] when `T` is not the type of the values or
    /// `out` reaches past the last one; [`ContainerError::Read`] when the source fails.
    pub fn read<T: Value>(&self, first: u64, out: &mut [T]) -> Result<(), ContainerError> {
        if T::TYPE != self.value_type {
            return Err(ContainerError::InvalidResult(
                "the result holds values of another type",
            ));
        }
        if first
            .checked_add(out.len() as u64)
            .is_none_or(|end| end > self.len)
        {
            return Err(ContainerError::InvalidResult(
                "a read past the last value of the result",
            ));
        }
        let size = T::TYPE.size();
        let mut buf = [0u8; CHUNK];
        let mut at = HEAD as u64 + first * size as u64;
        for values in out.chunks_mut(CHUNK / size) {
            let bytes = &mut buf[..values.len() * size];
            self.section.read_at(at, bytes)?;
            for (value, b) in values.iter_mut().zip(bytes.chunks_exact(size)) {
                *value = T::from_le(b);
            }
            at += bytes.len() as u64;
        }
        Ok(())
    }
}

impl<S: ByteSource> Container<S> {
    /// The identity of the graph this file holds (spec § 6).
    #[must_use]
    pub fn graph_identity(&self) -> GraphIdentity {
        GraphIdentity::of(self.entries())
    }

    /// The result stored under `key`, checked against its hash; `None` when there is
    /// none, or when `key` is of another graph than this file's (a result is served
    /// only for the graph it was computed on).
    ///
    /// # Errors
    ///
    /// [`ContainerError::Damaged`] when the result does not match its hash;
    /// [`ContainerError::InvalidResult`] for a result section that breaks its layout;
    /// [`ContainerError::Read`] and the errors of [`provenance`](Self::provenance).
    pub fn find_result(
        &self,
        key: &ResultKey,
    ) -> Result<Option<StoredResult<'_, S>>, ContainerError> {
        if key.graph != self.graph_identity() {
            return Ok(None);
        }
        let name = key.section_name()?;
        if self.get(&name).is_none_or(|e| e.kind != Kind::RESULT) {
            return Ok(None);
        }
        let Some(provenance) = self.provenance(&name)? else {
            return Ok(None);
        };
        if !key.matches(&provenance) {
            return Ok(None);
        }
        self.verify(&name)?;
        let section = self.section(&name)?;
        let mut head = [0u8; HEAD];
        section
            .read_at(0, &mut head)
            .map_err(|_| ContainerError::InvalidResult("shorter than its header"))?;
        if head[..8] != MAGIC {
            return Err(ContainerError::InvalidResult("no magic"));
        }
        let code = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
        let value_type = ValueType::from_code(code)
            .ok_or(ContainerError::InvalidResult("unknown value type"))?;
        let body = section.len() - HEAD as u64;
        let size = value_type.size() as u64;
        if !body.is_multiple_of(size) {
            return Err(ContainerError::InvalidResult(
                "not a whole number of values",
            ));
        }
        Ok(Some(StoredResult {
            section,
            value_type,
            len: body / size,
            provenance,
        }))
    }
}

impl<S: ByteSink> Writer<S> {
    /// The identity of the graph the next commit holds (spec § 6).
    #[must_use]
    pub fn graph_identity(&self) -> GraphIdentity {
        GraphIdentity::of(self.entries())
    }

    /// Appends `values` as the result of `key`, with a provenance record of its own; a
    /// result already stored under `key` is replaced. `about` gives the engine, the
    /// times and the inputs of the record; the operation and the parameters come from
    /// the key. Nothing is a stored result until [`commit`](Self::commit).
    ///
    /// # Errors
    ///
    /// [`ContainerError::InvalidResult`] when `key` is of another graph than the one
    /// the next commit holds, or when `about` names an operation or parameters; the
    /// errors of [`add_provenance`](Self::add_provenance) and
    /// [`SectionWriter::write`](super::SectionWriter::write).
    pub fn add_result<T: Value>(
        &mut self,
        key: &ResultKey,
        about: &Provenance,
        values: &[T],
    ) -> Result<Entry, ContainerError> {
        if key.graph != self.graph_identity() {
            return Err(ContainerError::InvalidResult(
                "the key is of another graph than this file's",
            ));
        }
        if !about.operation.is_empty() || !about.parameters.is_empty() {
            return Err(ContainerError::InvalidResult(
                "the operation and parameters of a result come from its key",
            ));
        }
        let name = key.section_name()?;
        let record = format!("{name}.provenance");
        // The section first: it refers to the record.
        for old in [&name, &record] {
            if self.get(old).is_some() {
                self.remove(old)?;
            }
        }
        let provenance = Provenance {
            operation: key.operation.clone(),
            parameters: key.parameters.clone(),
            ..about.clone()
        };
        self.add_provenance(&record, &provenance)?;
        let mut section = self.begin_section(&name, Kind::RESULT, Some(&record))?;
        let mut head = [0u8; HEAD];
        head[..8].copy_from_slice(&MAGIC);
        head[8..12].copy_from_slice(&T::TYPE.code().to_le_bytes());
        section.write(&head)?;
        let size = T::TYPE.size();
        let mut buf = [0u8; CHUNK];
        for chunk in values.chunks(CHUNK / size) {
            let bytes = &mut buf[..chunk.len() * size];
            for (value, b) in chunk.iter().zip(bytes.chunks_exact_mut(size)) {
                value.to_le(b);
            }
            section.write(bytes)?;
        }
        section.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_are_written_canonically() {
        let mut b = [0u8; 8];
        (-0.0_f64).to_le(&mut b);
        assert_eq!(b, 0.0_f64.to_le_bytes());
        f64::from_bits(0xFFF8_0000_0000_0001).to_le(&mut b);
        assert_eq!(u64::from_le_bytes(b), 0x7FF8_0000_0000_0000);
        let mut b = [0u8; 4];
        (-0.0_f32).to_le(&mut b);
        assert_eq!(b, [0; 4]);
        f32::from_bits(0xFFC0_0001).to_le(&mut b);
        assert_eq!(u32::from_le_bytes(b), 0x7FC0_0000);
        1.5_f32.to_le(&mut b);
        assert_eq!(f32::from_le(&b), 1.5);
    }

    #[test]
    fn the_identity_is_written_as_64_lowercase_digits() {
        let id = GraphIdentity([0xAB; 32]);
        assert_eq!(id.to_string(), "ab".repeat(32));
    }
}
