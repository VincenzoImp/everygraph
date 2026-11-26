// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A result as Arrow (`docs/api-sketch.md` § 3.4): value columns that borrow the
//! engine's arrays without a copy, and identifier columns made from the dictionary only
//! as the table is handed over, one batch at a time. Exported through the Arrow C
//! stream interface (`__arrow_c_stream__`), with the result's metadata under the key
//! `everygraph` of the schema.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::sync::Arc;

use arrow_array::ffi_stream::FFI_ArrowArrayStream;
use arrow_array::types::{Float64Type, UInt32Type};
use arrow_array::{
    Array, ArrayRef, ArrowPrimitiveType, PrimitiveArray, RecordBatch, RecordBatchReader,
    UInt32Array,
};
use arrow_buffer::{BooleanBuffer, Buffer, NullBuffer, ScalarBuffer, ToByteSlice};
use arrow_schema::ffi::FFI_ArrowSchema;
use arrow_schema::{ArrowError, DataType, Field, Schema, SchemaRef};
use everygraph_core::{Memory, MemoryError, MemoryKind, Reservation, SegVec};
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyCapsule, PyList, PyTuple};

use crate::graph::Shared;

/// Rows per batch of the stream: the identifiers of one batch are made at a time.
const BATCH_ROWS: usize = 1 << 20;

/// A segment of a shared array, as the bytes of an Arrow buffer.
struct Segment<T: Copy>(Arc<SegVec<T>>, usize);

impl<T: Copy + arrow_buffer::ArrowNativeType> AsRef<[u8]> for Segment<T> {
    fn as_ref(&self) -> &[u8] {
        self.0
            .segments()
            .nth(self.1)
            .unwrap_or_default()
            .to_byte_slice()
    }
}

/// An array owned with the reservation that holds it.
struct Held<T> {
    values: Vec<T>,
    _held: Reservation,
}

impl<T: arrow_buffer::ArrowNativeType> AsRef<[u8]> for Held<T> {
    fn as_ref(&self) -> &[u8] {
        self.values.to_byte_slice()
    }
}

fn primitive<A: ArrowPrimitiveType>(owner: impl AsRef<[u8]> + Send + 'static) -> ArrayRef {
    let buffer = Buffer::from(bytes::Bytes::from_owner(owner));
    let len = buffer.len() / size_of::<A::Native>();
    Arc::new(PrimitiveArray::<A>::new(
        ScalarBuffer::new(buffer, 0, len),
        None,
    ))
}

/// The chunks of a column that lends `values` to Arrow, one per segment, without a copy.
pub(crate) fn lend<A>(values: SegVec<A::Native>) -> Vec<ArrayRef>
where
    A: ArrowPrimitiveType,
    A::Native: Send + Sync,
{
    if values.is_empty() {
        return vec![Arc::new(PrimitiveArray::<A>::from_iter_values([]))];
    }
    let values = Arc::new(values);
    (0..values.segments().len())
        .map(|i| primitive::<A>(Segment(Arc::clone(&values), i)))
        .collect()
}

/// [`lend`] of `values`, in which `null`, when given, marks a null: each chunk's bitmap
/// is held under `memory`.
///
/// # Errors
///
/// A bitmap that does not fit the budget.
pub(crate) fn lend_u32(
    values: SegVec<u32>,
    null: Option<u32>,
    memory: &Memory,
) -> Result<Vec<ArrayRef>, MemoryError> {
    let Some(null) = null else {
        return Ok(lend::<UInt32Type>(values));
    };
    let mut nulls = Vec::new();
    for segment in values.segments() {
        let bytes = segment.len().div_ceil(8) as u64;
        let held = memory.reserve("null bitmap", MemoryKind::Anonymous, bytes)?;
        let bits = BooleanBuffer::collect_bool(segment.len(), |i| segment[i] != null);
        let owned = Buffer::from(bytes::Bytes::from_owner(Held {
            values: bits.inner().as_slice().to_vec(),
            _held: held,
        }));
        nulls.push(NullBuffer::new(BooleanBuffer::new(owned, 0, segment.len())));
    }
    Ok(lend::<UInt32Type>(values)
        .into_iter()
        .zip(nulls.into_iter().map(Some).chain(std::iter::repeat(None)))
        .map(|(chunk, nulls)| {
            let values = chunk
                .as_any()
                .downcast_ref::<UInt32Array>()
                .map(|a| a.values().clone());
            match (values, nulls.filter(|n| n.null_count() > 0)) {
                (Some(v), Some(n)) => Arc::new(UInt32Array::new(v, Some(n))) as ArrayRef,
                _ => chunk,
            }
        })
        .collect())
}

/// Which nodes an identifier column names, by their dense ids.
#[derive(Clone, Debug)]
pub(crate) enum Ids {
    /// Every node, in graph order.
    All,
    /// These nodes; [`NULL`] is null.
    Some(Arc<Vec<u32>>),
}

#[derive(Clone)]
enum Column {
    Values(Vec<ArrayRef>),
    Ids(Ids),
}

/// A result table.
#[pyclass(frozen, module = "everygraph._native")]
#[derive(Clone)]
pub(crate) struct Table {
    graph: Arc<Shared>,
    names: Vec<String>,
    columns: Vec<Column>,
    rows: usize,
}

/// The null of an identifier column.
pub(crate) const NULL: u32 = u32::MAX;

impl Table {
    /// One row per node: `node`, then `columns`, each of `graph`'s node count.
    pub(crate) fn per_node(graph: Arc<Shared>, columns: Vec<(&str, Vec<ArrayRef>)>) -> Self {
        let rows = graph.nodes();
        let mut table = Self {
            graph,
            names: vec!["node".into()],
            columns: vec![Column::Ids(Ids::All)],
            rows,
        };
        for (name, chunks) in columns {
            table.names.push(name.into());
            table.columns.push(Column::Values(chunks));
        }
        table
    }

    /// Adds an identifier column of `ids`.
    pub(crate) fn with_ids(mut self, name: &str, ids: Vec<u32>) -> Self {
        self.names.push(name.into());
        self.columns.push(Column::Ids(Ids::Some(Arc::new(ids))));
        self
    }

    fn data_type(&self, column: &Column) -> DataType {
        match column {
            Column::Values(chunks) => chunks
                .first()
                .map_or(DataType::Null, |c| c.data_type().clone()),
            Column::Ids(_) => self.graph.id_type(),
        }
    }

    fn schema(&self, meta: Option<String>) -> SchemaRef {
        let fields: Vec<Field> = self
            .names
            .iter()
            .zip(&self.columns)
            .map(|(name, column)| {
                let nullable = matches!(column, Column::Ids(Ids::Some(_)))
                    || matches!(column, Column::Values(c) if c.iter().any(|a| a.null_count() > 0));
                Field::new(name, self.data_type(column), nullable)
            })
            .collect();
        let metadata = meta
            .map(|m| [("everygraph".to_owned(), m)].into())
            .unwrap_or_default();
        Arc::new(Schema::new_with_metadata(fields, metadata))
    }

    /// The rows `start..end` of `column`, as one array.
    fn slice(&self, column: &Column, start: usize, end: usize) -> Result<ArrayRef, ArrowError> {
        match column {
            Column::Values(chunks) if start == end => Ok(chunks[0].slice(0, 0)),
            Column::Values(chunks) => {
                let mut parts = Vec::new();
                let mut at = 0;
                for chunk in chunks {
                    let (a, b) = (at, at + chunk.len());
                    if b > start && a < end {
                        let from = start.max(a);
                        parts.push(chunk.slice(from - a, end.min(b) - from));
                    }
                    at = b;
                }
                match parts.as_slice() {
                    [one] => Ok(Arc::clone(one)),
                    _ => {
                        let refs: Vec<&dyn Array> = parts.iter().map(AsRef::as_ref).collect();
                        arrow_select::concat::concat(&refs).map_err(|e| e.to_string())
                    }
                }
            }
            Column::Ids(Ids::All) => self
                .graph
                .keys((start..end).map(|i| u32::try_from(i).unwrap_or(NULL))),
            Column::Ids(Ids::Some(ids)) => self.graph.keys(ids[start..end].iter().copied()),
        }
        .map_err(|e| ArrowError::ExternalError(e.into()))
    }

    fn batch(
        &self,
        schema: &SchemaRef,
        start: usize,
        end: usize,
    ) -> Result<RecordBatch, ArrowError> {
        let columns = self
            .columns
            .iter()
            .map(|c| self.slice(c, start, end))
            .collect::<Result<_, _>>()?;
        RecordBatch::try_new(Arc::clone(schema), columns)
    }

    /// The rows `rows` of every column, in that order; a row [`NULL`] is a row of nulls.
    pub(crate) fn take(&self, rows: &[u32]) -> PyResult<Self> {
        let columns = self
            .columns
            .iter()
            .map(|column| match column {
                Column::Values(chunks) => {
                    // The chunks, and after them one null: the source of a null row. Rows are
                    // picked from where they are, so no column is ever joined or copied.
                    let null = arrow_array::new_null_array(chunks[0].data_type(), 1);
                    let mut starts = Vec::with_capacity(chunks.len());
                    let mut at = 0;
                    for c in chunks {
                        starts.push(at);
                        at += c.len();
                    }
                    let picks: Vec<(usize, usize)> = rows
                        .iter()
                        .map(|&r| {
                            if r == NULL {
                                return (chunks.len(), 0);
                            }
                            let r = r as usize;
                            let chunk = starts.partition_point(|&s| s <= r) - 1;
                            (chunk, r - starts[chunk])
                        })
                        .collect();
                    let mut refs: Vec<&dyn Array> = chunks.iter().map(AsRef::as_ref).collect();
                    refs.push(null.as_ref());
                    Ok(Column::Values(vec![arrow_select::interleave::interleave(
                        &refs, &picks,
                    )?]))
                }
                Column::Ids(Ids::All) => Ok(Column::Ids(Ids::Some(Arc::new(rows.to_vec())))),
                Column::Ids(Ids::Some(ids)) => Ok(Column::Ids(Ids::Some(Arc::new(
                    rows.iter()
                        .map(|&r| if r == NULL { NULL } else { ids[r as usize] })
                        .collect(),
                )))),
            })
            .collect::<Result<_, ArrowError>>()
            .map_err(arrow_err)?;
        Ok(Self {
            graph: Arc::clone(&self.graph),
            names: self.names.clone(),
            columns,
            rows: rows.len(),
        })
    }

    fn values(&self, name: &str) -> PyResult<&[ArrayRef]> {
        match self
            .names
            .iter()
            .position(|n| n == name)
            .map(|i| &self.columns[i])
        {
            Some(Column::Values(chunks)) => Ok(chunks),
            Some(Column::Ids(_)) => Err(PyValueError::new_err(format!("{name} holds identifiers"))),
            None => Err(PyKeyError::new_err(name.to_owned())),
        }
    }
}

/// A value that orders rows: numbers by value, nulls left out.
#[derive(Clone, Copy, PartialEq)]
struct Key(f64);

impl Eq for Key {}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Calls `f(row, value)` for every non-null value of `chunks`, as `f64` (exact below
/// 2^53).
fn each_value(chunks: &[ArrayRef], mut f: impl FnMut(u32, f64)) -> PyResult<()> {
    use arrow_array::cast::AsArray;
    use arrow_array::types::{Float32Type, UInt64Type};
    fn visit<A: ArrowPrimitiveType>(
        array: &PrimitiveArray<A>,
        row: &mut u32,
        to: impl Fn(A::Native) -> f64,
        f: &mut impl FnMut(u32, f64),
    ) {
        for v in array {
            if let Some(v) = v {
                f(*row, to(v));
            }
            *row += 1;
        }
    }
    let mut row = 0_u32;
    for chunk in chunks {
        match chunk.data_type() {
            DataType::Float64 => {
                visit(chunk.as_primitive::<Float64Type>(), &mut row, |v| v, &mut f);
            }
            DataType::Float32 => {
                visit(
                    chunk.as_primitive::<Float32Type>(),
                    &mut row,
                    f64::from,
                    &mut f,
                );
            }
            DataType::UInt32 => visit(
                chunk.as_primitive::<UInt32Type>(),
                &mut row,
                f64::from,
                &mut f,
            ),
            #[expect(
                clippy::cast_precision_loss,
                reason = "an order of counts, exact below 2^53"
            )]
            DataType::UInt64 => visit(
                chunk.as_primitive::<UInt64Type>(),
                &mut row,
                |v| v as f64,
                &mut f,
            ),
            t => {
                return Err(PyValueError::new_err(format!(
                    "cannot order a column of {t}"
                )));
            }
        }
    }
    Ok(())
}

#[pymethods]
impl Table {
    /// Rows.
    #[getter]
    fn num_rows(&self) -> usize {
        self.rows
    }

    /// Column names, in order.
    #[getter]
    fn columns(&self) -> Vec<String> {
        self.names.clone()
    }

    /// The first `n` rows.
    fn head(&self, n: usize) -> PyResult<Self> {
        let rows: Vec<u32> = (0..self.rows.min(n)).map(row).collect();
        self.take(&rows)
    }

    /// The `n` rows with the largest value of `by`, largest first, ties in table order,
    /// rows whose value is null left out (`docs/api-sketch.md` § 2.12), with a column
    /// `rank` from 1.
    fn top(&self, py: Python<'_>, n: usize, by: &str) -> PyResult<Self> {
        let chunks = self.values(by)?;
        let picked = py.detach(|| {
            let mut heap = BinaryHeap::with_capacity(n.saturating_add(1).min(self.rows + 1));
            each_value(chunks, |row, v| {
                heap.push(Reverse((Key(v), Reverse(row))));
                if heap.len() > n {
                    heap.pop();
                }
            })?;
            let mut rows: Vec<(Key, Reverse<u32>)> = heap.into_iter().map(|Reverse(x)| x).collect();
            rows.sort_unstable_by(|a, b| b.cmp(a));
            Ok::<_, PyErr>(
                rows.into_iter()
                    .map(|(_, Reverse(r))| r)
                    .collect::<Vec<u32>>(),
            )
        })?;
        let mut table = self.take(&picked)?;
        let ranks: Vec<u32> = (1..=picked.len()).map(row).collect();
        table.names.push("rank".into());
        table
            .columns
            .push(Column::Values(vec![Arc::new(UInt32Array::from(ranks))]));
        Ok(table)
    }

    /// One row per value of `column` (unsigned integers) with its number of rows, as
    /// `nodes`: largest first, equal sizes by value.
    fn sizes(&self, py: Python<'_>, column: &str) -> PyResult<Self> {
        let chunks = self.values(column)?;
        let values: Vec<&UInt32Array> = chunks
            .iter()
            .map(|c| c.as_any().downcast_ref::<UInt32Array>())
            .collect::<Option<_>>()
            .ok_or_else(|| PyValueError::new_err(format!("{column} is not uint32")))?;
        // Components and cores are numbered densely from 0: one count per number.
        let largest = values.iter().filter_map(|v| v.iter().flatten().max()).max();
        let len = largest.map_or(0, |m| m as usize + 1);
        let _held = self
            .graph
            .memory()
            .reserve("sizes", MemoryKind::Anonymous, len as u64 * 12)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let mut counts = vec![0_u32; len];
        py.detach(|| {
            for v in values.iter().flat_map(|a| a.iter().flatten()) {
                counts[v as usize] += 1;
            }
        });
        let mut numbers: Vec<u32> = (0..len)
            .map(row)
            .filter(|&v| counts[v as usize] > 0)
            .collect();
        numbers.sort_by_key(|&v| (Reverse(counts[v as usize]), v));
        let sizes: Vec<u32> = numbers.iter().map(|&v| counts[v as usize]).collect();
        Ok(Self {
            graph: Arc::clone(&self.graph),
            names: vec![column.into(), "nodes".into()],
            rows: numbers.len(),
            columns: vec![
                Column::Values(vec![Arc::new(UInt32Array::from(numbers))]),
                Column::Values(vec![Arc::new(UInt32Array::from(sizes))]),
            ],
        })
    }

    /// The rows `start..stop` as tuples of Python values: for display, a few at a time.
    fn rows<'py>(
        &self,
        py: Python<'py>,
        start: usize,
        stop: usize,
    ) -> PyResult<Bound<'py, PyList>> {
        let stop = stop.min(self.rows);
        let start = start.min(stop);
        let schema = self.schema(None);
        let batch = self.batch(&schema, start, stop).map_err(arrow_err)?;
        let columns: Vec<Vec<Py<PyAny>>> = batch
            .columns()
            .iter()
            .map(|c| (0..c.len()).map(|i| scalar(py, c.as_ref(), i)).collect())
            .collect::<PyResult<_>>()?;
        let rows = (0..stop - start).map(|i| {
            PyTuple::new(py, columns.iter().map(|c| c[i].clone_ref(py))).map(Bound::unbind)
        });
        PyList::new(py, rows.collect::<PyResult<Vec<_>>>()?)
    }

    /// The Arrow schema, with `meta` (JSON) under the key `everygraph`.
    #[pyo3(signature = (meta=None))]
    fn schema_capsule<'py>(
        &self,
        py: Python<'py>,
        meta: Option<String>,
    ) -> PyResult<Bound<'py, PyCapsule>> {
        let schema = FFI_ArrowSchema::try_from(self.schema(meta).as_ref()).map_err(arrow_err)?;
        PyCapsule::new_with_value(py, schema, c"arrow_schema")
    }

    /// The Arrow C stream of the table, in batches, with `meta` (JSON) under the key
    /// `everygraph` of its schema.
    #[pyo3(signature = (meta=None))]
    fn stream_capsule<'py>(
        &self,
        py: Python<'py>,
        meta: Option<String>,
    ) -> PyResult<Bound<'py, PyCapsule>> {
        let schema = self.schema(meta);
        let reader = Batches {
            table: self.clone(),
            schema,
            at: 0,
        };
        let stream = FFI_ArrowArrayStream::new(Box::new(reader));
        PyCapsule::new_with_value(py, stream, c"arrow_array_stream")
    }
}

/// A row number: below 2^32, as a table has one row per node at most.
fn row(r: usize) -> u32 {
    u32::try_from(r).unwrap_or(u32::MAX)
}

fn arrow_err(e: ArrowError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// One value of `array`, as a Python object.
fn scalar(py: Python<'_>, array: &dyn Array, i: usize) -> PyResult<Py<PyAny>> {
    use arrow_array::cast::AsArray;
    use arrow_array::types::{Float32Type, Int64Type, UInt64Type};
    if array.is_null(i) {
        return Ok(py.None());
    }
    let any = match array.data_type() {
        DataType::Float64 => array
            .as_primitive::<Float64Type>()
            .value(i)
            .into_pyobject(py)?
            .into_any(),
        DataType::Float32 => array
            .as_primitive::<Float32Type>()
            .value(i)
            .into_pyobject(py)?
            .into_any(),
        DataType::UInt32 => array
            .as_primitive::<UInt32Type>()
            .value(i)
            .into_pyobject(py)?
            .into_any(),
        DataType::UInt64 => array
            .as_primitive::<UInt64Type>()
            .value(i)
            .into_pyobject(py)?
            .into_any(),
        DataType::Int64 => array
            .as_primitive::<Int64Type>()
            .value(i)
            .into_pyobject(py)?
            .into_any(),
        DataType::Utf8 => array
            .as_string::<i32>()
            .value(i)
            .into_pyobject(py)?
            .into_any(),
        DataType::Binary => array
            .as_binary::<i32>()
            .value(i)
            .into_pyobject(py)?
            .into_any(),
        DataType::FixedSizeBinary(_) => array
            .as_fixed_size_binary()
            .value(i)
            .into_pyobject(py)?
            .into_any(),
        other => {
            return Err(PyValueError::new_err(format!(
                "cannot show a value of {other}"
            )));
        }
    };
    Ok(any.unbind())
}

/// The stream of a table's batches.
struct Batches {
    table: Table,
    schema: SchemaRef,
    at: usize,
}

impl Iterator for Batches {
    type Item = Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.at >= self.table.rows {
            return None;
        }
        let end = (self.at + BATCH_ROWS).min(self.table.rows);
        let batch = self.table.batch(&self.schema, self.at, end);
        self.at = end;
        Some(batch)
    }
}

impl RecordBatchReader for Batches {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}
