// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reading every value of a column: what `verify --deep` checks beyond § 7 of
//! `docs/spec/eg-columns.md`, which leaves the values to the section hash.

use std::collections::BTreeMap;

use everygraph_core::le::array;
use everygraph_core::math::{canonical, canonical_f32};
use everygraph_core::{ByteSource, Memory, MemoryKind, ReadError, widen};

use super::stats::{Accumulator, STATS_LEN, magnitude, pow10, widen as sign_extend};
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
            let v = i128::from_le_bytes(array(&sign_extend(wide, ty), 0));
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
/// from them; the rows of a column of an adjacency's scope against the facts of that
/// adjacency; a column of an in-adjacency against the same column of its
/// out-adjacency (the same statistics); and what the build derives from the events,
/// against the columns and the adjacency (§ 6: `multiplicity.max`, and `edges`, `first`
/// and `last` under `duplicates="sum"`). The layout of one column at a time is held, reserved in
/// `memory`.
pub(crate) fn verify_columns<S: ByteSource>(
    file: &Container<S>,
    adjacencies: &[AdjacencyFacts],
    memory: &Memory,
    report: &mut Report,
) -> Result<(), ReadError> {
    let has = |name: &str, kind: Kind| file.get(name).is_some_and(|e| e.kind == kind);
    let mut checked = BTreeMap::new();
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
        let (rows, stats) = match check_column(file, &names, column, memory) {
            Ok(Some(found)) => found,
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
        if let Some((adj, part @ ("pair" | "event"))) = scope.rsplit_once('.') {
            match adjacencies.iter().find(|a| a.name == adj) {
                Some(facts) => {
                    let expected = match part {
                        "pair" => facts.entries,
                        _ => facts.events.unwrap_or(facts.entries),
                    };
                    if rows != expected {
                        report.problem(
                            Some(name),
                            format!("{rows} rows, but its adjacency gives its scope {expected}"),
                        );
                    }
                }
                None if has(&format!("{adj}.index"), Kind::ADJACENCY_INDEX) => {
                    report.unchecked.push(format!(
                        "{name}: its rows (its adjacency {adj} was not checked)"
                    ));
                }
                None => report.problem(
                    Some(name),
                    format!("its scope names the adjacency {adj}, which the file does not hold"),
                ),
            }
        }
        checked.insert((scope.to_owned(), column.to_owned()), stats);
    }
    // A column of an in-adjacency holds the values of the same column of its
    // out-adjacency, in another order: the same statistics.
    for ((scope, column), stats) in &checked {
        let Some((base, part)) = scope.rsplit_once('.') else {
            continue;
        };
        let Some(base) = base.strip_suffix(".in") else {
            continue;
        };
        let out = (format!("{base}.out.{part}"), column.clone());
        let name = SectionNames::new(scope, column).column;
        match checked.get(&out) {
            None => report.problem(
                Some(&name),
                format!(
                    "its out-adjacency has no column {}",
                    SectionNames::new(&out.0, column).column
                ),
            ),
            Some(theirs) => {
                let encoded = |s: &Option<ColumnStats>| s.as_ref().map(ColumnStats::encode);
                if encoded(stats) != encoded(theirs) {
                    report.problem(
                        Some(&name),
                        "its values are not those of the same column of its out-adjacency",
                    );
                }
            }
        }
    }
    // Each provenance record is decoded once, however many adjacencies refer to it.
    let mut declared = BTreeMap::new();
    for facts in adjacencies {
        let index = format!("{}.index", facts.name);
        let Some(record) = file.get(&index).and_then(|e| e.provenance.clone()) else {
            continue;
        };
        let parameters = declared
            .entry(record)
            .or_insert_with_key(|record| file.read_provenance(record).ok().map(|p| p.parameters));
        if let Some(parameters) = parameters {
            check_derived(file, facts, parameters, &checked, memory, report)?;
        }
    }
    Ok(())
}

/// The statistics of each column that was checked, by scope and name.
type Checked = BTreeMap<(String, String), Option<ColumnStats>>;

/// `multiplicity.max` of `parameters`, the provenance of adjacency `facts`, and under
/// `duplicates="sum"` its `edges` (at least 1), `first` and `last` (never after).
fn check_derived<S: ByteSource>(
    file: &Container<S>,
    facts: &AdjacencyFacts,
    parameters: &BTreeMap<String, String>,
    checked: &Checked,
    memory: &Memory,
    report: &mut Report,
) -> Result<(), ReadError> {
    let index = format!("{}.index", facts.name);
    let Some(declared) = parameters.get("multiplicity.max") else {
        return Ok(());
    };
    let scope = format!("{}.pair", facts.name);
    let get = |column: &str| checked.get(&(scope.clone(), column.to_owned()));
    let as_u64 = |b: [u8; 32]| u64::from_le_bytes(array(&b, 0));
    let sum = parameters.get("duplicates").map(String::as_str) == Some("sum");
    let expected = if sum {
        let Some(edges) = get("edges") else {
            report.problem(
                Some(&index),
                format!("the graph sums its duplicates, and {scope}/arrow/edges is missing"),
            );
            return Ok(());
        };
        let range = edges.as_ref().and_then(|s| s.range);
        if range.is_some_and(|(min, _)| as_u64(min) == 0) {
            report.problem(Some(&format!("{scope}/arrow/edges")), "a pair of 0 edges");
        }
        range.map_or(0, |(_, max)| as_u64(max))
    } else {
        facts.multiplicity_max
    };
    if declared.parse::<u64>().ok() != Some(expected) {
        report.problem(
            Some(&index),
            format!("its provenance gives multiplicity.max {declared}, its pairs {expected}"),
        );
    }
    if sum && get("first").is_some() && get("last").is_some() {
        let first = SectionNames::new(&scope, "first");
        match first_after_last(file, &first, &SectionNames::new(&scope, "last"), memory) {
            Ok(Some(Some(row))) => report.problem(
                Some(&first.column),
                format!("row {row}: the first time is after the last"),
            ),
            Ok(Some(None)) => {}
            Ok(None) => report.unchecked.push(format!(
                "{scope}: first and last (not enough memory for their layouts)"
            )),
            Err(ColumnError::Read(e @ ReadError::Backend(_))) => return Err(e),
            Err(e) => report.problem(Some(&first.column), e.to_string()),
        }
    }
    Ok(())
}

/// The first row at which column `first` holds a later time than column `last`, two
/// timestamp columns of the same rows without nulls, already checked; `None` when their
/// layouts do not fit `memory`.
fn first_after_last<S: ByteSource>(
    file: &Container<S>,
    first: &SectionNames,
    last: &SectionNames,
    memory: &Memory,
) -> Result<Option<Option<u64>>, ColumnError> {
    // The two layouts, already checked below 16 MiB, each read and parsed.
    let len = |names: &SectionNames| file.get(&names.layout).map_or(0, |e| e.len);
    let bytes = (len(first) + len(last)).saturating_mul(2);
    let Ok(_held) = memory.reserve("verify", MemoryKind::Anonymous, bytes) else {
        return Ok(None);
    };
    let layout = |names: &SectionNames| -> Result<ColumnLayout, ColumnError> {
        let section = file.section(&names.layout)?;
        let mut bytes = vec![0; small(section.len())];
        section.read_at(0, &mut bytes)?;
        ColumnLayout::parse(&bytes)
    };
    let (a, b) = (layout(first)?, layout(last)?);
    let nulls = a
        .batches()
        .iter()
        .chain(b.batches())
        .any(|x| x.null_count > 0);
    if a.rows() != b.rows() || a.batch_rows() != b.batch_rows() || a.ty() != b.ty() || nulls {
        return Err(ColumnError::Invalid(
            "first and last are not timestamps of the same rows without nulls",
        ));
    }
    let (sa, sb) = (file.section(&first.column)?, file.section(&last.column)?);
    let (mut x, mut y) = (Vec::new(), Vec::new());
    let mut left = a.rows();
    let mut row = 0;
    for (ba, bb) in a.batches().iter().zip(b.batches()) {
        let rows = left.min(a.batch_rows());
        left -= rows;
        for start in (0..rows).step_by(small(ROWS)) {
            let n = ROWS.min(rows - start);
            x.resize(small(n * 8), 0);
            y.resize(small(n * 8), 0);
            sa.read_at(ba.values.start + start * 8, &mut x)?;
            sb.read_at(bb.values.start + start * 8, &mut y)?;
            let time = |c: &[u8]| i64::from_le_bytes(array(c, 0));
            let later = x
                .chunks_exact(8)
                .zip(y.chunks_exact(8))
                .position(|(p, q)| time(p) > time(q));
            if let Some(i) = later {
                return Ok(Some(Some(row + widen(i))));
            }
            row += n;
        }
    }
    Ok(Some(None))
}

/// Checks one column, its layout and its statistics; its rows and the statistics of its
/// values, or `None` when its layout does not fit `memory`.
fn check_column<S: ByteSource>(
    file: &Container<S>,
    names: &SectionNames,
    column: &str,
    memory: &Memory,
) -> Result<Option<(u64, Option<ColumnStats>)>, ColumnError> {
    let read = |name: &str| -> Result<Vec<u8>, ColumnError> {
        let section = file.section(name)?;
        let mut bytes = vec![0; small(section.len())];
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
    match (&stats, file.get(&names.stats)) {
        (None, None) => {}
        (Some(computed), Some(entry)) if entry.kind == Kind::COLUMN_STATS => {
            // Its length first: a reader allocates nothing larger than the record.
            if entry.len != widen(STATS_LEN) {
                return Err(ColumnError::Invalid(
                    "a statistics section that is not 112 bytes",
                ));
            }
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
    Ok(Some((layout.rows(), stats)))
}
