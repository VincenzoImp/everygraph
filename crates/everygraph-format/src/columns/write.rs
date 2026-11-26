// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Writing a column into a container (`docs/spec/eg-columns.md` § 3–5).

use std::ops::Range;

use everygraph_core::ByteSink;
use everygraph_core::math::{canonical, canonical_f32};

use super::arrow::{Plan, Shape};
use super::layout::{self, ColumnLayout};
use super::stats::{Accumulator, ColumnStats, magnitude, pow10};
use super::{CodeWidth, ColumnError, ColumnType, SectionNames};
use crate::container::{ContainerError, Kind, MAX_NAME_BYTES, SectionWriter, Writer};

/// Bytes gathered before each write to the sink.
const STAGING: usize = 1 << 16;
const ZEROS: [u8; 4096] = [0; 4096];

/// What a column is: its name, type, rows and how they are cut into batches.
#[derive(Clone, Copy, Debug)]
pub struct ColumnSpec<'a> {
    /// The column's name, also its Arrow field name: 1 to 512 bytes.
    pub name: &'a str,
    /// Its type.
    pub ty: ColumnType,
    /// Its number of rows.
    pub rows: u64,
    /// Whether it may hold nulls.
    pub nullable: bool,
    /// Rows per batch: a power of two up to 2^30, with each batch below 2 GiB.
    pub batch_rows: u64,
    /// The strings of a dictionary-encoded column.
    pub dictionary: Option<Dictionary<'a>>,
}

impl<'a> ColumnSpec<'a> {
    /// A column that is not nullable, in batches of [`ColumnType::default_batch_rows`].
    #[must_use]
    pub fn new(name: &'a str, ty: ColumnType, rows: u64) -> Self {
        Self {
            name,
            ty,
            rows,
            nullable: false,
            batch_rows: ty.default_batch_rows(),
            dictionary: None,
        }
    }

    fn shape(&self) -> Shape {
        Shape {
            name: self.name.to_owned(),
            ty: self.ty,
            nullable: self.nullable,
            rows: self.rows,
            batch_rows: self.batch_rows,
            dictionary: self.dictionary.map(|d| (d.values(), d.bytes.len() as u64)),
        }
    }
}

/// The strings of a dictionary-encoded column as Arrow keeps them: string `i` is
/// `bytes[offsets[i] .. offsets[i + 1]]`, so `offsets` has one more element than there
/// are strings, starts at 0 and ends at `bytes.len()`.
#[derive(Clone, Copy, Debug)]
pub struct Dictionary<'a> {
    /// Where each string starts, then where the last ends.
    pub offsets: &'a [u32],
    /// The strings' UTF-8 bytes, concatenated.
    pub bytes: &'a [u8],
}

impl Dictionary<'_> {
    /// The offsets and bytes of `strings`, for [`Dictionary`].
    #[must_use]
    pub fn parts(strings: &[&str]) -> (Vec<u32>, Vec<u8>) {
        let mut offsets = vec![0];
        let mut bytes = Vec::new();
        for s in strings {
            bytes.extend_from_slice(s.as_bytes());
            offsets.push(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
        }
        (offsets, bytes)
    }

    /// The number of strings.
    #[must_use]
    pub fn values(&self) -> u64 {
        self.offsets.len().saturating_sub(1) as u64
    }

    fn check(&self) -> Result<(), ColumnError> {
        let bad = ColumnError::Input(
            "dictionary offsets that do not start at 0, decrease, or do not end at the \
             bytes' length, or strings that are not UTF-8",
        );
        let (Some(&0), Some(&last)) = (self.offsets.first(), self.offsets.last()) else {
            return Err(bad);
        };
        if usize::try_from(last).ok() != Some(self.bytes.len()) {
            return Err(bad);
        }
        for w in self.offsets.windows(2) {
            let range =
                usize::try_from(w[0]).unwrap_or(usize::MAX)..usize::try_from(w[1]).unwrap_or(0);
            match self.bytes.get(range) {
                Some(s) if std::str::from_utf8(s).is_ok() => {}
                _ => return Err(bad),
            }
        }
        Ok(())
    }
}

/// The validity of some rows: row `i` (for `i` in `0..len`) is valid when bit
/// `offset + i` of `bytes` is set (bit `j % 8` of byte `j / 8`). A batch's validity
/// may come in several pieces, such as the segments of a bit array.
#[derive(Clone, Copy, Debug)]
pub struct Bits<'a> {
    /// The bitmap.
    pub bytes: &'a [u8],
    /// The first bit used.
    pub offset: u64,
    /// The number of rows.
    pub len: u64,
}

impl Bits<'_> {
    fn iter(&self) -> impl Iterator<Item = bool> + '_ {
        (self.offset..self.offset + self.len).map(|j| {
            usize::try_from(j / 8)
                .ok()
                .and_then(|i| self.bytes.get(i))
                .is_some_and(|b| b >> (j % 8) & 1 == 1)
        })
    }

    fn fits(&self) -> bool {
        self.offset
            .checked_add(self.len)
            .is_some_and(|end| end <= self.bytes.len() as u64 * 8)
    }
}

macro_rules! values {
    ($($variant:ident($t:ty)),* $(,)?) => {
        /// Values handed to [`ColumnWriter::write_batch`]: a slice of the Rust type of
        /// the column's type.
        ///
        /// `I64` serves `Int64` and timestamps; `U8`, `U16`, `U32` also serve the codes
        /// of a dictionary-encoded column; `I128` is decimal128; `Bytes` holds
        /// `fixed_size_binary` and decimal256 values (32 bytes of two's complement,
        /// little-endian), concatenated.
        #[derive(Clone, Copy, Debug)]
        #[non_exhaustive]
        pub enum Values<'a> {
            $(
                #[allow(missing_docs)]
                $variant(&'a [$t]),
            )*
            #[allow(missing_docs)]
            Bytes(&'a [u8]),
        }

        $(
            impl<'a> From<&'a [$t]> for Values<'a> {
                fn from(v: &'a [$t]) -> Self {
                    Self::$variant(v)
                }
            }

            impl<'a, const N: usize> From<&'a [$t; N]> for Values<'a> {
                fn from(v: &'a [$t; N]) -> Self {
                    Self::$variant(v)
                }
            }
        )*

        impl<'a> Values<'a> {
            /// Elements of the slice (bytes for `Bytes`).
            fn len(&self) -> usize {
                match self {
                    $(Self::$variant(v) => v.len(),)*
                    Self::Bytes(v) => v.len(),
                }
            }

            fn slice(&self, r: Range<usize>) -> Values<'a> {
                match self {
                    $(Self::$variant(v) => Self::$variant(&v[r]),)*
                    Self::Bytes(v) => Self::Bytes(&v[r]),
                }
            }
        }
    };
}

values! {
    I8(i8), I16(i16), I32(i32), I64(i64),
    U8(u8), U16(u16), U32(u32), U64(u64),
    F32(f32), F64(f64), Bool(bool), I128(i128),
}

impl Values<'_> {
    /// Whether these values fit a column of type `ty`.
    fn fits(&self, ty: ColumnType) -> bool {
        use ColumnType as T;
        match (self, ty) {
            (Self::Bytes(b), T::FixedSizeBinary(_) | T::Decimal256 { .. }) => {
                (b.len() as u64).is_multiple_of(ty.width())
            }
            _ => matches!(
                (self, ty),
                (Self::I8(_), T::Int8)
                    | (Self::I16(_), T::Int16)
                    | (Self::I32(_), T::Int32)
                    | (Self::I64(_), T::Int64 | T::Timestamp(_))
                    | (Self::U8(_), T::UInt8 | T::Dictionary(CodeWidth::U8))
                    | (Self::U16(_), T::UInt16 | T::Dictionary(CodeWidth::U16))
                    | (Self::U32(_), T::UInt32 | T::Dictionary(CodeWidth::U32))
                    | (Self::U64(_), T::UInt64)
                    | (Self::F32(_), T::Float32)
                    | (Self::F64(_), T::Float64)
                    | (Self::Bool(_), T::Bool)
                    | (Self::I128(_), T::Decimal128 { .. })
            ),
        }
    }

    /// Rows of a column of type `ty` these values make.
    fn rows(&self, ty: ColumnType) -> u64 {
        match self {
            Self::Bytes(b) => b.len() as u64 / ty.width().max(1),
            other => other.len() as u64,
        }
    }
}

/// Writes one column into a container: the Arrow IPC file as it goes, batch by batch;
/// then, at [`finish`](Self::finish), the layout and (for numeric columns) the
/// statistics.
///
/// The bytes written are a function of the spec, the values and the validity alone
/// (no threads, no clock). Floats are stored canonically: NaN as the quiet NaN
/// `0x7FC00000` or `0x7FF8000000000000`, −0.0 as +0.0. A null row stores zero bytes.
/// Nothing of a batch is held in memory: bytes go to the sink through a 64 KiB buffer.
#[derive(Debug)]
pub struct ColumnWriter<'w, S: ByteSink> {
    section: SectionWriter<'w, S>,
    names: SectionNames,
    provenance: Option<String>,
    plan: Plan,
    nulls: Vec<u64>,
    stats: Option<Accumulator>,
    staging: Vec<u8>,
    /// The bits of a bitmap byte not yet staged, and their count.
    bits: (u8, u32),
    /// The batch begun by [`begin_batch`](Self::begin_batch): its end in the section
    /// and the rows it still needs.
    open: Option<(u64, u64)>,
}

impl<'w, S: ByteSink> ColumnWriter<'w, S> {
    /// Starts column `spec.name` of scope `scope` (`nodes`, `edges`, `<adj>.pair`,
    /// `<adj>.event`; § 6) in `writer`, made as the provenance record `provenance`
    /// says. Writes the Arrow file's schema and, for a dictionary-encoded column, its
    /// dictionary.
    ///
    /// # Errors
    ///
    /// [`ColumnError::Input`] for a spec that breaks the limits of § 2 and § 3.1, a
    /// scope that is empty or holds a `/`, or a dictionary that is missing, given to a
    /// column of another type, or malformed (see [`Dictionary`]);
    /// [`ColumnError::TooLarge`] for a column whose layout would exceed 16 MiB;
    /// [`ColumnError::Container`] when a section name is taken or invalid, or the sink
    /// fails.
    pub fn begin(
        writer: &'w mut Writer<S>,
        scope: &str,
        spec: &ColumnSpec<'_>,
        provenance: Option<&str>,
    ) -> Result<Self, ColumnError> {
        if scope.is_empty() || scope.contains('/') {
            return Err(ColumnError::Input("a scope is not empty and holds no '/'"));
        }
        let shape = spec.shape();
        layout::check(&shape)?;
        if let Some(d) = &spec.dictionary {
            d.check()?;
        }
        let names = SectionNames::new(scope, spec.name);
        for name in [&names.column, &names.layout, &names.stats] {
            if name.len() > MAX_NAME_BYTES {
                return Err(ContainerError::InvalidName.into());
            }
            if writer.get(name).is_some() {
                return Err(ContainerError::NameTaken(name.clone()).into());
            }
        }
        let section = writer.begin_section(&names.column, Kind::COLUMN, provenance)?;
        let (plan, head) = Plan::new(shape);
        let mut this = Self {
            section,
            names,
            provenance: provenance.map(str::to_owned),
            stats: spec.ty.is_numeric().then(|| Accumulator::new(spec.ty)),
            plan,
            nulls: Vec::new(),
            staging: Vec::with_capacity(STAGING),
            bits: (0, 0),
            open: None,
        };
        this.section.write(&head)?;
        if let Some(d) = spec.dictionary {
            this.write_dictionary(d)?;
        }
        Ok(this)
    }

    fn write_dictionary(&mut self, d: Dictionary<'_>) -> Result<(), ColumnError> {
        let frame = self.plan.dictionary();
        self.section.write(&frame.metadata)?;
        self.fill_to(frame.buffers[1].start)?;
        // Below 2^31 (`Shape::check`), so the same bytes as Arrow's `i32` offsets.
        for o in d.offsets {
            self.stage(&o.to_le_bytes())?;
        }
        self.flush()?;
        // Empty strings only: the data buffer is empty, at offset 0 of the body.
        if !frame.buffers[2].is_empty() {
            self.fill_to(frame.buffers[2].start)?;
            self.section.write(d.bytes)?;
        }
        self.fill_to(frame.end)
    }

    /// Appends the next batch: exactly [`ColumnSpec::batch_rows`] rows (fewer for the
    /// last batch), given as one or more slices in row order. `validity` covers the
    /// batch's rows in order, in one or more pieces, for a nullable column only; empty
    /// means no null. The values of null rows are ignored.
    ///
    /// # Errors
    ///
    /// [`ColumnError::Input`] for values of another type, the wrong number of rows,
    /// validity that does not cover the batch, lies past its bytes or is given to a
    /// column that is not nullable, a dictionary code that names no string, a decimal
    /// with more digits than its precision, or a batch after the last;
    /// [`ColumnError::Container`] when the sink fails.
    pub fn write_batch(
        &mut self,
        values: &[Values<'_>],
        validity: &[Bits<'_>],
    ) -> Result<(), ColumnError> {
        let shape = self.plan.shape();
        let (ty, index) = (shape.ty, self.nulls.len() as u64);
        if index >= shape.batches() || self.open.is_some() {
            return Err(ColumnError::Input(
                "a batch after the last, or inside another",
            ));
        }
        let rows = shape.batch_len(index);
        if values.iter().any(|v| !v.fits(ty))
            || values.iter().map(|v| v.rows(ty)).sum::<u64>() != rows
        {
            return Err(ColumnError::Input(
                "values of another type or count than the batch's",
            ));
        }
        if !validity.is_empty()
            && (!shape.nullable
                || validity.iter().any(|b| !b.fits())
                || validity.iter().map(|b| b.len).sum::<u64>() != rows)
        {
            return Err(ColumnError::Input(
                "validity that does not cover the batch, lies past its bytes, or is given \
                 to a column that is not nullable",
            ));
        }
        let valid = || {
            validity
                .iter()
                .flat_map(Bits::iter)
                .chain(std::iter::repeat(true))
        };
        let nulls = valid()
            .take(usize::try_from(rows).unwrap_or(usize::MAX))
            .filter(|&v| !v)
            .count() as u64;
        let dictionary = shape.dictionary.map_or(0, |(values, _)| values);
        check_values(values, ty, dictionary, valid())?;

        let frame = self.plan.batch(rows, nulls);
        self.section.write(&frame.metadata)?;
        self.fill_to(frame.buffers[0].start)?;
        if nulls > 0 {
            for v in validity.iter().flat_map(Bits::iter) {
                self.push_bit(v)?;
            }
            self.end_bits()?;
            self.fill_to(frame.buffers[1].start)?;
        }
        if let Some(stats) = &mut self.stats {
            stats.null(nulls);
        }
        let mut valid = valid();
        for chunk in values {
            self.write_values(chunk, ty, &mut valid)?;
        }
        self.end_bits()?;
        self.fill_to(frame.end)?;
        self.nulls.push(nulls);
        Ok(())
    }

    /// Begins the next batch, with no null, whose values then come in pieces of any
    /// size through [`push`](Self::push), so that a batch never has to be held in
    /// memory; [`end_batch`](Self::end_batch) ends it.
    ///
    /// # Errors
    ///
    /// [`ColumnError::Input`] for a batch after the last or inside another;
    /// [`ColumnError::Container`] when the sink fails.
    pub fn begin_batch(&mut self) -> Result<(), ColumnError> {
        let shape = self.plan.shape();
        let index = self.nulls.len() as u64;
        if index >= shape.batches() || self.open.is_some() {
            return Err(ColumnError::Input(
                "a batch after the last, or inside another",
            ));
        }
        let rows = shape.batch_len(index);
        let frame = self.plan.batch(rows, 0);
        self.section.write(&frame.metadata)?;
        self.fill_to(frame.buffers[1].start)?;
        self.open = Some((frame.end, rows));
        Ok(())
    }

    /// The next values of the batch begun by [`begin_batch`](Self::begin_batch).
    ///
    /// # Errors
    ///
    /// [`ColumnError::Input`] outside a batch, for values of another type, more rows
    /// than the batch has left, a dictionary code that names no string or a decimal with
    /// more digits than its precision; [`ColumnError::Container`] when the sink fails.
    pub fn push(&mut self, values: Values<'_>) -> Result<(), ColumnError> {
        let shape = self.plan.shape();
        let (ty, dictionary) = (shape.ty, shape.dictionary.map_or(0, |(v, _)| v));
        let rows = values.rows(ty);
        let left = match self.open {
            Some((_, left)) if values.fits(ty) && rows <= left => left,
            _ => {
                return Err(ColumnError::Input(
                    "values outside a batch, of another type, or past its rows",
                ));
            }
        };
        check_values(&[values], ty, dictionary, std::iter::repeat(true))?;
        self.write_values(&values, ty, &mut std::iter::repeat(true))?;
        self.open = self.open.map(|(end, _)| (end, left - rows));
        Ok(())
    }

    /// Ends the batch begun by [`begin_batch`](Self::begin_batch).
    ///
    /// # Errors
    ///
    /// [`ColumnError::Input`] outside a batch or before it has its rows;
    /// [`ColumnError::Container`] when the sink fails.
    pub fn end_batch(&mut self) -> Result<(), ColumnError> {
        let Some((end, 0)) = self.open else {
            return Err(ColumnError::Input(
                "a batch ended outside a batch or before its rows",
            ));
        };
        self.end_bits()?;
        self.fill_to(end)?;
        self.open = None;
        self.nulls.push(0);
        Ok(())
    }

    /// Writes one slice of values; `valid` gives the validity of each row in turn.
    fn write_values(
        &mut self,
        chunk: &Values<'_>,
        ty: ColumnType,
        valid: &mut impl Iterator<Item = bool>,
    ) -> Result<(), ColumnError> {
        macro_rules! each {
            ($xs:expr, |$x:ident| $bytes:expr, $observe:expr) => {
                for &$x in $xs {
                    let stored = $bytes;
                    if valid.next().unwrap_or(true) {
                        if let Some(stats) = &mut self.stats {
                            let mut wide = [0u8; 32];
                            wide[..stored.len()].copy_from_slice(&stored);
                            $observe(stats, wide);
                        }
                        self.stage(&stored)?;
                    } else {
                        self.stage(&[0; 16][..stored.len()])?;
                    }
                }
            };
        }
        let int = |s: &mut Accumulator, x: i128, w| s.int(x, w);
        match *chunk {
            Values::I8(xs) => each!(xs, |x| x.to_le_bytes(), |s, w| int(s, x.into(), w)),
            Values::I16(xs) => each!(xs, |x| x.to_le_bytes(), |s, w| int(s, x.into(), w)),
            Values::I32(xs) => each!(xs, |x| x.to_le_bytes(), |s, w| int(s, x.into(), w)),
            Values::I64(xs) => each!(xs, |x| x.to_le_bytes(), |s, w| int(s, x.into(), w)),
            Values::U8(xs) => each!(xs, |x| x.to_le_bytes(), |s, w| int(s, x.into(), w)),
            Values::U16(xs) => each!(xs, |x| x.to_le_bytes(), |s, w| int(s, x.into(), w)),
            Values::U32(xs) => each!(xs, |x| x.to_le_bytes(), |s, w| int(s, x.into(), w)),
            Values::U64(xs) => each!(xs, |x| x.to_le_bytes(), |s, w| int(s, x.into(), w)),
            Values::I128(xs) => each!(xs, |x| x.to_le_bytes(), |s, w| int(s, x, w)),
            Values::F32(xs) => each!(
                xs,
                |x| canonical_f32(x).to_le_bytes(),
                |s: &mut Accumulator, w| s.float(f64::from(canonical_f32(x)), w)
            ),
            Values::F64(xs) => each!(
                xs,
                |x| canonical(x).to_le_bytes(),
                |s: &mut Accumulator, w| s.float(canonical(x), w)
            ),
            Values::Bool(xs) => {
                for &x in xs {
                    let v = valid.next().unwrap_or(true);
                    self.push_bit(x && v)?;
                }
            }
            Values::Bytes(bytes) => {
                let width = usize::try_from(ty.width()).unwrap_or(usize::MAX);
                for x in bytes.chunks_exact(width) {
                    if valid.next().unwrap_or(true) {
                        if let Some(stats) = &mut self.stats {
                            let mut wide = [0u8; 32];
                            wide.copy_from_slice(x);
                            stats.wide(wide);
                        }
                        self.stage(x)?;
                    } else {
                        self.fill(width as u64)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn push_bit(&mut self, bit: bool) -> Result<(), ColumnError> {
        self.bits.0 |= u8::from(bit) << self.bits.1;
        self.bits.1 += 1;
        if self.bits.1 == 8 {
            self.end_bits()?;
        }
        Ok(())
    }

    /// Stages the partial bitmap byte, if any; its bits past the last row are zero.
    fn end_bits(&mut self) -> Result<(), ColumnError> {
        if self.bits.1 > 0 {
            let byte = self.bits.0;
            self.bits = (0, 0);
            self.stage(&[byte])?;
        }
        Ok(())
    }

    fn stage(&mut self, bytes: &[u8]) -> Result<(), ColumnError> {
        self.staging.extend_from_slice(bytes);
        if self.staging.len() >= STAGING {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), ColumnError> {
        self.section.write(&self.staging)?;
        self.staging.clear();
        Ok(())
    }

    fn fill(&mut self, mut n: u64) -> Result<(), ColumnError> {
        while n > 0 {
            let k = n.min(ZEROS.len() as u64);
            self.stage(&ZEROS[..usize::try_from(k).unwrap_or(0)])?;
            n -= k;
        }
        Ok(())
    }

    /// Zero bytes up to `target` in the section.
    fn fill_to(&mut self, target: u64) -> Result<(), ColumnError> {
        self.flush()?;
        let at = self.section.len();
        self.fill(target - at)?;
        self.flush()
    }

    /// Writes the end of the Arrow file, then the layout and the statistics sections.
    /// Returns the statistics, for a numeric column.
    ///
    /// # Errors
    ///
    /// [`ColumnError::Input`] when fewer batches were written than the rows need;
    /// [`ColumnError::Container`] when the sink fails.
    pub fn finish(mut self) -> Result<Option<ColumnStats>, ColumnError> {
        if self.nulls.len() as u64 != self.plan.shape().batches() || self.open.is_some() {
            return Err(ColumnError::Input("fewer batches than the rows need"));
        }
        let tail = self.plan.tail();
        self.section.write(&tail)?;
        let Self {
            section,
            names,
            provenance,
            plan,
            nulls,
            stats,
            ..
        } = self;
        let (_, writer) = section.finish_into_writer()?;
        let layout = ColumnLayout::new(plan.shape().clone(), &nulls)?;
        let provenance = provenance.as_deref();
        writer.add_section(
            &names.layout,
            Kind::COLUMN_LAYOUT,
            &layout.encode(),
            provenance,
        )?;
        let stats = stats.map(Accumulator::finish);
        if let Some(s) = &stats {
            writer.add_section(&names.stats, Kind::COLUMN_STATS, &s.encode(), provenance)?;
        }
        Ok(stats)
    }
}

/// Refuses dictionary codes past the dictionary and decimals past their precision,
/// before anything of the batch is written. Null rows are not checked.
fn check_values(
    values: &[Values<'_>],
    ty: ColumnType,
    dictionary: u64,
    mut valid: impl Iterator<Item = bool>,
) -> Result<(), ColumnError> {
    let mut ok = true;
    let mut each = |good: bool| {
        let row_is_valid = valid.next().unwrap_or(true);
        ok &= good || !row_is_valid;
    };
    for chunk in values {
        match (*chunk, ty) {
            (Values::U8(c), ColumnType::Dictionary(_)) => {
                c.iter().for_each(|&c| each(u64::from(c) < dictionary));
            }
            (Values::U16(c), ColumnType::Dictionary(_)) => {
                c.iter().for_each(|&c| each(u64::from(c) < dictionary));
            }
            (Values::U32(c), ColumnType::Dictionary(_)) => {
                c.iter().for_each(|&c| each(u64::from(c) < dictionary));
            }
            (Values::I128(v), ColumnType::Decimal128 { precision, .. }) => {
                let limit = 10u128.pow(u32::from(precision));
                v.iter().for_each(|&x| each(x.unsigned_abs() < limit));
            }
            (Values::Bytes(b), ColumnType::Decimal256 { precision, .. }) => {
                let limit = pow10(precision);
                b.chunks_exact(32)
                    .for_each(|x| each(magnitude(x.try_into().unwrap_or([0; 32])) < limit));
            }
            _ => return Ok(()),
        }
    }
    if ok {
        Ok(())
    } else {
        Err(ColumnError::Input(
            "a dictionary code that names no string, or a decimal with more digits than its precision",
        ))
    }
}

/// Writes a whole column held in memory, batch by batch: `values` has `spec.rows`
/// rows, none of them null.
///
/// # Errors
///
/// As [`ColumnWriter::begin`] and [`ColumnWriter::write_batch`].
pub fn write_column<'a, S: ByteSink>(
    writer: &mut Writer<S>,
    scope: &str,
    spec: &ColumnSpec<'_>,
    values: impl Into<Values<'a>>,
    provenance: Option<&str>,
) -> Result<Option<ColumnStats>, ColumnError> {
    let values = values.into();
    if !values.fits(spec.ty) || values.rows(spec.ty) != spec.rows {
        return Err(ColumnError::Input(
            "values of another type or count than the column's",
        ));
    }
    let per_row = match values {
        Values::Bytes(_) => usize::try_from(spec.ty.width()).unwrap_or(usize::MAX),
        _ => 1,
    };
    let step = usize::try_from(spec.batch_rows)
        .unwrap_or(usize::MAX)
        .saturating_mul(per_row);
    let mut column = ColumnWriter::begin(writer, scope, spec, provenance)?;
    let mut at = 0;
    while at < values.len() {
        let end = values.len().min(at.saturating_add(step));
        column.write_batch(&[values.slice(at..end)], &[])?;
        at = end;
    }
    column.finish()
}
