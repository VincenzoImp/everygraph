// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reading every value of a column: what `verify --deep` checks beyond § 7 of
//! `docs/spec/eg-columns.md`, which leaves the values to the section hash.

use everygraph_core::le::array;
use everygraph_core::math::{canonical, canonical_f32};
use everygraph_core::{ByteSource, Memory, MemoryKind, ReadError};

use super::stats::{Accumulator, magnitude, pow10, widen};
use super::{
    Column, ColumnError, ColumnLayout, ColumnStats, ColumnType, MAX_LAYOUT_BYTES, SectionNames,
};
use crate::container::layout::small;
use crate::container::{Container, Kind};
use crate::verify::{AdjacencyFacts, Report};

/// Rows read at a time: a multiple of 8, so that a piece of a bitmap is whole bytes.
const ROWS: u64 = 1 << 13;

impl<S: ByteSource> Column<'_, S> {
    /// Reads every value and checks what [`open`](Self::open) does not: floats are
    /// canonical, null rows are zero, dictionary codes name a string, decimals fit
    /// their precision. Returns the statistics the values imply, for a numeric column,
    /// to compare with the stored ones.
    ///
    /// # Errors
    ///
    /// [`ColumnError::Invalid`] for a value that breaks one of these rules;
    /// [`ColumnError::Read`] when the source fails.
    pub fn check_values(&self) -> Result<Option<ColumnStats>, ColumnError> {
        let layout = self.layout();
        let ty = layout.ty();
        let width = ty.width();
        let codes = layout.dictionary().map(|d| d.values);
        let mut stats = ty.is_numeric().then(|| Accumulator::new(ty));
        let (mut values, mut validity) = (Vec::new(), Vec::new());
        let mut left = layout.rows();
        for batch in layout.batches() {
            let rows = left.min(layout.batch_rows());
            left -= rows;
            if let Some(s) = &mut stats {
                s.null(batch.null_count);
            }
            for first in (0..rows).step_by(small(ROWS)) {
                let n = ROWS.min(rows - first);
                validity.clear();
                if batch.null_count > 0 {
                    validity.resize(small(n.div_ceil(8)), 0);
                    self.source()
                        .read_at(batch.validity.start + first / 8, &mut validity)?;
                }
                let valid =
                    |i: u64| validity.is_empty() || validity[small(i / 8)] >> (i % 8) & 1 == 1;
                if ty == ColumnType::Bool {
                    values.resize(small(n.div_ceil(8)), 0);
                    self.source()
                        .read_at(batch.values.start + first / 8, &mut values)?;
                    if (0..n).any(|i| !valid(i) && values[small(i / 8)] >> (i % 8) & 1 == 1) {
                        return Err(ColumnError::Invalid("a null row that holds a value"));
                    }
                    continue;
                }
                values.resize(small(n * width), 0);
                self.source()
                    .read_at(batch.values.start + first * width, &mut values)?;
                for (i, x) in (0..).zip(values.chunks_exact(small(width))) {
                    if !valid(i) {
                        if x.iter().any(|&b| b != 0) {
                            return Err(ColumnError::Invalid("a null row that holds a value"));
                        }
                        continue;
                    }
                    check_value(ty, x, codes, stats.as_mut())?;
                }
            }
        }
        Ok(stats.map(Accumulator::finish))
    }
}

/// Checks one stored value of `ty` and adds it to `stats`.
fn check_value(
    ty: ColumnType,
    x: &[u8],
    codes: Option<u64>,
    stats: Option<&mut Accumulator>,
) -> Result<(), ColumnError> {
    let mut wide = [0u8; 32];
    wide[..x.len()].copy_from_slice(x);
    let fits = match ty {
        ColumnType::Float32 => {
            let v = f32::from_le_bytes(array(x, 0));
            if let Some(s) = stats {
                s.float(f64::from(v), wide);
            }
            canonical_f32(v).to_bits() == v.to_bits()
        }
        ColumnType::Float64 => {
            let v = f64::from_le_bytes(array(x, 0));
            if let Some(s) = stats {
                s.float(v, wide);
            }
            canonical(v).to_bits() == v.to_bits()
        }
        ColumnType::Dictionary(_) => {
            let code = u64::from_le_bytes(array(&wide, 0));
            codes.is_some_and(|values| code < values)
        }
        ColumnType::FixedSizeBinary(_) | ColumnType::Bool => true,
        ColumnType::Decimal256 { precision, .. } => {
            if let Some(s) = stats {
                s.wide(wide);
            }
            magnitude(wide) < pow10(precision)
        }
        _ => {
            let v = i128::from_le_bytes(array(&widen(wide, ty), 0));
            if let Some(s) = stats {
                s.int(v, wide);
            }
            match ty {
                ColumnType::Decimal128 { precision, .. } => {
                    v.unsigned_abs() < 10u128.pow(u32::from(precision))
                }
                _ => true,
            }
        }
    };
    if fits {
        Ok(())
    } else {
        Err(ColumnError::Invalid(
            "a float that is not canonical, a code that names no string, or a decimal \
             past its precision",
        ))
    }
}

/// The column sections of `file`, for `verify` (`docs/spec/eg-columns.md` § 6, § 7):
/// each column with its layout and, when numeric, its statistics, under the names of
/// § 6; the framing against the layout; every value read and the statistics recomputed
/// from them; and the rows of a column of an adjacency's scope against the facts of
/// that adjacency. The layout of one column at a time is held, reserved in `memory`.
pub(crate) fn verify_columns<S: ByteSource>(
    file: &Container<S>,
    adjacencies: &[AdjacencyFacts],
    memory: &Memory,
    report: &mut Report,
) -> Result<(), ReadError> {
    let has = |name: &str, kind: Kind| file.get(name).is_some_and(|e| e.kind == kind);
    for e in file.entries() {
        let name = &*e.name;
        let role = match e.kind {
            Kind::COLUMN => "arrow",
            Kind::COLUMN_LAYOUT => "layout",
            Kind::COLUMN_STATS => "stats",
            _ => continue,
        };
        let Some((scope, column)) = name
            .split_once('/')
            .and_then(|(scope, rest)| Some((scope, rest.strip_prefix(role)?.strip_prefix('/')?)))
            .filter(|(scope, column)| !scope.is_empty() && !column.is_empty())
        else {
            report.problem(
                Some(name),
                format!("a column section whose name is not <scope>/{role}/<column>"),
            );
            continue;
        };
        let names = SectionNames::new(scope, column);
        if e.kind != Kind::COLUMN {
            if !has(&names.column, Kind::COLUMN) {
                report.problem(Some(name), format!("there is no column {}", names.column));
            }
            continue;
        }
        if !has(&names.layout, Kind::COLUMN_LAYOUT) {
            report.problem(Some(name), format!("there is no layout {}", names.layout));
            continue;
        }
        let rows = match check_column(file, &names, column, memory) {
            Ok(Some(rows)) => rows,
            Ok(None) => {
                report.unchecked.push(format!(
                    "{name}: its values (not enough memory for its layout)"
                ));
                continue;
            }
            Err(ColumnError::Read(e @ ReadError::Backend(_))) => return Err(e),
            Err(e) => {
                report.problem(Some(name), e.to_string());
                continue;
            }
        };
        let expected = scope.rsplit_once('.').and_then(|(adj, part)| {
            let facts = adjacencies.iter().find(|a| a.name == adj)?;
            match part {
                "pair" => Some(facts.entries),
                "event" => Some(facts.events.unwrap_or(facts.entries)),
                _ => None,
            }
        });
        if let Some(expected) = expected.filter(|&r| r != rows) {
            report.problem(
                Some(name),
                format!("{rows} rows, but its adjacency gives its scope {expected}"),
            );
        }
    }
    Ok(())
}

/// Checks one column, its layout and its statistics; its rows, or `None` when its
/// layout does not fit `memory`.
fn check_column<S: ByteSource>(
    file: &Container<S>,
    names: &SectionNames,
    column: &str,
    memory: &Memory,
) -> Result<Option<u64>, ColumnError> {
    let read = |name: &str| -> Result<Vec<u8>, ColumnError> {
        let section = file.section(name)?;
        let mut bytes = vec![0; usize::try_from(section.len()).unwrap_or(usize::MAX)];
        section.read_at(0, &mut bytes)?;
        Ok(bytes)
    };
    let len = file.get(&names.layout).map_or(0, |e| e.len);
    if len > MAX_LAYOUT_BYTES {
        return Err(ColumnError::Invalid("a layout section over 16 MiB"));
    }
    let Ok(_held) = memory.reserve("verify", MemoryKind::Anonymous, len.saturating_mul(2)) else {
        return Ok(None);
    };
    let layout = ColumnLayout::parse(&read(&names.layout)?)?;
    if layout.name() != column {
        return Err(ColumnError::Invalid(
            "its layout names another column than its section",
        ));
    }
    let values = Column::open(&layout, file.section(&names.column)?)?;
    let stats = values.check_values()?;
    match (stats, file.get(&names.stats)) {
        (None, None) => {}
        (Some(computed), Some(entry)) if entry.kind == Kind::COLUMN_STATS => {
            let stored = read(&names.stats)?;
            ColumnStats::parse(&stored, &layout)?;
            if computed.encode()[..] != stored[..] {
                return Err(ColumnError::Invalid(
                    "its statistics are not those of its values",
                ));
            }
        }
        (Some(_), _) => return Err(ColumnError::Invalid("a numeric column without statistics")),
        (None, Some(_)) => {
            return Err(ColumnError::Invalid(
                "statistics of a column that is not numeric",
            ));
        }
    }
    Ok(Some(layout.rows()))
}
