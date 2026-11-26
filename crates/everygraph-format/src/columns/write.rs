// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Writing a column into a container (`docs/spec/eg-columns.md` § 3–5).

use std::ops::Range;

use everygraph_core::ByteSink;

use super::arrow::{Plan, Shape};
use super::layout::{self, ColumnLayout};
use super::stats::{Accumulator, ColumnStats};
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
    /// The strings of a dictionary-encoded column, in code order.
    pub dictionary: Option<&'a [&'a str]>,
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
            dictionary: self
                .dictionary
                .map(|d| (d.len() as u64, d.iter().map(|s| s.len() as u64).sum())),
        }
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
        matches!(
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
                | (Self::Bytes(_), T::FixedSizeBinary(_) | T::Decimal256 { .. })
        )
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
#[derive(Debug)]
pub struct ColumnWriter<'w, S: ByteSink> {
    section: SectionWriter<'w, S>,
    names: SectionNames,
    provenance: Option<String>,
    plan: Plan,
    nulls: Vec<u64>,
    stats: Option<Accumulator>,
    staging: Vec<u8>,
}

impl<'w, S: ByteSink> ColumnWriter<'w, S> {
    /// Starts column `spec.name` of scope `scope` (`nodes`, `edges`, `<adj>.pair`,
    /// `<adj>.event`; § 6) in `writer`, made as the provenance record `provenance`
    /// says. Writes the Arrow file's schema and, for a dictionary-encoded column, its
    /// dictionary.
    ///
    /// # Errors
    ///
    /// [`ColumnError::Input`] for a spec that breaks the limits of § 2 and § 3.1, or a
    /// dictionary given to a column of another type (or missing);
    /// [`ColumnError::TooLarge`] for a column whose layout would exceed 16 MiB;
    /// [`ColumnError::Container`] when a section name is taken or invalid, or the sink
    /// fails.
    pub fn begin(
        writer: &'w mut Writer<S>,
        scope: &str,
        spec: &ColumnSpec<'_>,
        provenance: Option<&str>,
    ) -> Result<Self, ColumnError> {
        let shape = spec.shape();
        layout::check(&shape)?;
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
        };
        this.section.write(&head)?;
        if let Some(strings) = spec.dictionary {
            this.write_dictionary(strings)?;
        }
        Ok(this)
    }

    fn write_dictionary(&mut self, strings: &[&str]) -> Result<(), ColumnError> {
        let frame = self.plan.dictionary();
        self.section.write(&frame.metadata)?;
        self.fill_to(frame.buffers[1].start)?;
        let mut end = 0i32;
        self.stage(&end.to_le_bytes())?;
        for s in strings {
            // The dictionary body is below 2 GiB (`Shape::check`).
            end += i32::try_from(s.len()).unwrap_or(i32::MAX);
            self.stage(&end.to_le_bytes())?;
        }
        self.flush()?;
        // Empty strings only: the data buffer is empty, at offset 0 of the body.
        if !frame.buffers[2].is_empty() {
            self.fill_to(frame.buffers[2].start)?;
        }
        for s in strings {
            self.stage(s.as_bytes())?;
        }
        self.flush()?;
        self.fill_to(frame.end)
    }

    /// Appends the next batch: exactly [`ColumnSpec::batch_rows`] rows (fewer for the
    /// last batch), given as one or more slices in row order. `validity` is the batch's
    /// bitmap (bit `i % 8` of byte `i / 8` set when row `i` is not null; bits past the
    /// last row ignored), for a nullable column only; `None` means no null. The values
    /// of null rows are ignored.
    ///
    /// # Errors
    ///
    /// [`ColumnError::Input`] for values of another type, the wrong number of rows, a
    /// validity bitmap of the wrong length or on a column that is not nullable, a
    /// dictionary code that names no string, a decimal with more digits than its
    /// precision, or a batch after the last; [`ColumnError::Container`] when the sink
    /// fails.
    pub fn write_batch(
        &mut self,
        values: &[Values<'_>],
        validity: Option<&[u8]>,
    ) -> Result<(), ColumnError> {
        let shape = self.plan.shape();
        let (ty, index) = (shape.ty, self.nulls.len() as u64);
        if index >= shape.batches() {
            return Err(ColumnError::Input("a batch after the last"));
        }
        let rows = shape.batch_len(index);
        if values.iter().any(|v| !v.fits(ty))
            || values.iter().map(|v| v.rows(ty)).sum::<u64>() != rows
            || values.iter().any(
                |v| matches!(v, Values::Bytes(b) if !(b.len() as u64).is_multiple_of(ty.width())),
            )
        {
            return Err(ColumnError::Input(
                "values of another type or count than the batch's",
            ));
        }
        let bitmap_len = usize::try_from(rows.div_ceil(8)).unwrap_or(usize::MAX);
        if validity.is_some_and(|v| v.len() != bitmap_len || !shape.nullable) {
            return Err(ColumnError::Input(
                "a validity bitmap of the wrong length, or on a column that is not nullable",
            ));
        }
        let valid = |r: u64| validity.is_none_or(|v| bit(v, r));
        let nulls = (0..rows).filter(|&r| !valid(r)).count() as u64;
        let dictionary = shape.dictionary.map_or(0, |(values, _)| values);
        self.check_values(values, ty, dictionary, &valid)?;

        let frame = self.plan.batch(rows, nulls);
        self.section.write(&frame.metadata)?;
        self.fill_to(frame.buffers[0].start)?;
        if let (Some(bits), true) = (validity, nulls > 0) {
            let (last, whole) = bits.split_last().unwrap_or((&0, &[]));
            self.stage(whole)?;
            // Bits past the last row are written as zero.
            let used = rows % 8;
            self.stage(&[if used == 0 {
                *last
            } else {
                last & ((1 << used) - 1)
            }])?;
            self.flush()?;
            self.fill_to(frame.buffers[1].start)?;
        }
        if let Some(stats) = &mut self.stats {
            stats.null(nulls);
        }
        let mut row = 0u64;
        let mut bits = (0u8, 0u32);
        for chunk in values {
            row = self.write_values(chunk, ty, row, &valid, &mut bits)?;
        }
        if bits.1 > 0 {
            self.stage(&[bits.0])?;
        }
        self.flush()?;
        self.fill_to(frame.end)?;
        self.nulls.push(nulls);
        Ok(())
    }

    /// Refuses dictionary codes past the dictionary and decimals past their precision,
    /// before anything of the batch is written.
    fn check_values(
        &self,
        values: &[Values<'_>],
        ty: ColumnType,
        dictionary: u64,
        valid: &impl Fn(u64) -> bool,
    ) -> Result<(), ColumnError> {
        let mut row = 0u64;
        let mut ok = true;
        let mut each = |good: bool| {
            ok &= !valid(row) || good;
            row += 1;
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
                    let limit = pow10_256(precision);
                    b.chunks_exact(32)
                        .for_each(|x| each(magnitude_256(x) < limit));
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

    /// Writes one slice of values from batch row `row` on; returns the row after it.
    /// `bits` is the partial byte of a `bool` column and its bit count.
    fn write_values(
        &mut self,
        chunk: &Values<'_>,
        ty: ColumnType,
        mut row: u64,
        valid: &impl Fn(u64) -> bool,
        bits: &mut (u8, u32),
    ) -> Result<u64, ColumnError> {
        macro_rules! each {
            ($xs:expr, |$x:ident| $bytes:expr, $observe:expr) => {
                for &$x in $xs {
                    let stored = $bytes;
                    if valid(row) {
                        if let Some(stats) = &mut self.stats {
                            let mut wide = [0u8; 32];
                            wide[..stored.len()].copy_from_slice(&stored);
                            $observe(stats, wide);
                        }
                        self.stage(&stored)?;
                    } else {
                        self.stage(&[0; 16][..stored.len()])?;
                    }
                    row += 1;
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
                |x| canonical_f64(x).to_le_bytes(),
                |s: &mut Accumulator, w| s.float(canonical_f64(x), w)
            ),
            Values::Bool(xs) => {
                for &x in xs {
                    bits.0 |= u8::from(x && valid(row)) << bits.1;
                    bits.1 += 1;
                    if bits.1 == 8 {
                        self.stage(&[bits.0])?;
                        *bits = (0, 0);
                    }
                    row += 1;
                }
            }
            Values::Bytes(bytes) => {
                let width = usize::try_from(ty.width()).unwrap_or(usize::MAX);
                for x in bytes.chunks_exact(width) {
                    if !valid(row) {
                        self.fill(width as u64)?;
                    } else {
                        if let Some(stats) = &mut self.stats {
                            let mut wide = [0u8; 32];
                            wide.copy_from_slice(x);
                            stats.wide(wide);
                        }
                        self.stage(x)?;
                    }
                    row += 1;
                }
            }
        }
        Ok(row)
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
        if self.nulls.len() as u64 != self.plan.shape().batches() {
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
    if values.rows(spec.ty) != spec.rows {
        return Err(ColumnError::Input(
            "values of another count than the column's rows",
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
        column.write_batch(&[values.slice(at..end)], None)?;
        at = end;
    }
    column.finish()
}

/// Bit `r % 8` of byte `r / 8`.
fn bit(bitmap: &[u8], r: u64) -> bool {
    usize::try_from(r / 8)
        .ok()
        .and_then(|i| bitmap.get(i))
        .is_some_and(|b| b >> (r % 8) & 1 == 1)
}

/// The quiet NaN for every NaN, +0.0 for −0.0 (eg-container § 1).
fn canonical_f32(x: f32) -> f32 {
    if x.is_nan() {
        f32::from_bits(0x7FC0_0000)
    } else if x == 0.0 {
        0.0
    } else {
        x
    }
}

fn canonical_f64(x: f64) -> f64 {
    if x.is_nan() {
        f64::from_bits(0x7FF8_0000_0000_0000)
    } else if x == 0.0 {
        0.0
    } else {
        x
    }
}

/// 10^p as a 256-bit integer: (high 128 bits, low 128 bits). For p ≤ 76 it is below
/// 2^253.
fn pow10_256(p: u8) -> (u128, u128) {
    let (mut hi, mut lo) = (0u128, 1u128);
    for _ in 0..p {
        // (hi, lo) × 10, from 64-bit halves of `lo`.
        let lo_lo = (lo & u128::from(u64::MAX)) * 10;
        let lo_hi = (lo >> 64) * 10 + (lo_lo >> 64);
        lo = (lo_hi << 64) | (lo_lo & u128::from(u64::MAX));
        hi = hi * 10 + (lo_hi >> 64);
    }
    (hi, lo)
}

/// The magnitude of a 256-bit two's complement integer, little-endian, as
/// (high 128 bits, low 128 bits).
fn magnitude_256(le: &[u8]) -> (u128, u128) {
    let mut lo = [0; 16];
    let mut hi = [0; 16];
    lo.copy_from_slice(&le[..16]);
    hi.copy_from_slice(&le[16..32]);
    let (lo, hi) = (u128::from_le_bytes(lo), u128::from_le_bytes(hi));
    if hi >> 127 == 0 {
        return (hi, lo);
    }
    let (lo, carry) = (!lo).overflowing_add(1);
    (!hi + u128::from(carry), lo)
}
