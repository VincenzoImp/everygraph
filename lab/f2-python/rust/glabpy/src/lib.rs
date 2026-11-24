use arrow_array::{Array, ArrayRef, Float64Array, StructArray, UInt32Array};
use arrow_buffer::ScalarBuffer;
use arrow_data::ffi::FFI_ArrowArray;
use arrow_schema::ffi::FFI_ArrowSchema;
use arrow_schema::{DataType, Field};
use glabcore::Csr;
use numpy::{PyArray1, PyReadonlyArray1};
use pyo3::exceptions::{PyIndexError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyCapsule, PyList, PyTuple};
use std::sync::Arc;

fn with_threads<T: Send>(threads: Option<usize>, f: impl FnOnce() -> T + Send) -> T {
    match threads {
        Some(t) if t > 0 => rayon::ThreadPoolBuilder::new().num_threads(t).build().unwrap().install(f),
        _ => f(),
    }
}

/// An Arrow array exported through the Arrow PyCapsule interface (zero-copy).
#[pyclass(frozen, module = "glabpy._glab")]
struct ArrowArray {
    arr: ArrayRef,
    field: Field,
}

#[pymethods]
impl ArrowArray {
    #[pyo3(signature = (requested_schema=None))]
    fn __arrow_c_array__<'py>(&self, py: Python<'py>, requested_schema: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyTuple>> {
        let _ = requested_schema;
        let schema = FFI_ArrowSchema::try_from(&self.field).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let array = FFI_ArrowArray::new(&self.arr.to_data());
        let sc = PyCapsule::new_with_value(py, schema, c"arrow_schema")?;
        let ac = PyCapsule::new_with_value(py, array, c"arrow_array")?;
        PyTuple::new(py, [sc.into_any(), ac.into_any()])
    }
    fn __arrow_c_schema__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyCapsule>> {
        let schema = FFI_ArrowSchema::try_from(&self.field).map_err(|e| PyValueError::new_err(e.to_string()))?;
        PyCapsule::new_with_value(py, schema, c"arrow_schema")
    }
    fn __len__(&self) -> usize { self.arr.len() }
    /// Address of the first value buffer (for zero-copy verification).
    fn buffer_addresses(&self) -> Vec<usize> {
        fn rec(d: &arrow_data::ArrayData, out: &mut Vec<usize>) {
            for b in d.buffers() { out.push(b.as_ptr() as usize); }
            for c in d.child_data() { rec(c, out); }
        }
        let mut v = vec![]; rec(&self.arr.to_data(), &mut v); v
    }
}

fn f64_arrow(v: Vec<f64>, name: &str) -> (ArrayRef, Field) {
    let a = Float64Array::new(ScalarBuffer::from(v), None); // Vec moved, not copied
    (Arc::new(a), Field::new(name, DataType::Float64, false))
}
fn u32_arrow(v: Vec<u32>, name: &str) -> (ArrayRef, Field) {
    let a = UInt32Array::new(ScalarBuffer::from(v), None);
    (Arc::new(a), Field::new(name, DataType::UInt32, false))
}

#[pyclass(frozen, module = "glabpy._glab")]
struct Graph {
    g: Arc<Csr>,
}

#[pymethods]
impl Graph {
    #[staticmethod]
    fn open(path: &str) -> PyResult<Graph> {
        Ok(Graph { g: Arc::new(Csr::open(path).map_err(|e| PyValueError::new_err(e.to_string()))?) })
    }
    /// Build from an undirected edge list (u32 numpy arrays). GIL released, parallel.
    #[staticmethod]
    #[pyo3(signature = (src, dst, n, threads=None))]
    fn from_edges(py: Python<'_>, src: PyReadonlyArray1<u32>, dst: PyReadonlyArray1<u32>, n: usize, threads: Option<usize>) -> PyResult<Graph> {
        let s = src.as_slice()?; let d = dst.as_slice()?;
        if let (Some(a), Some(b)) = (s.iter().max(), d.iter().max()) {
            if (*a.max(b) as usize) >= n { return Err(PyValueError::new_err("node id >= n")); }
        }
        let g = py.detach(|| with_threads(threads, || Csr::from_edges(s, d, n)));
        Ok(Graph { g: Arc::new(g) })
    }
    #[getter] fn n(&self) -> usize { self.g.n() }
    #[getter] fn entries(&self) -> usize { self.g.entries() }

    fn degree<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<u32>> {
        let d = py.detach(|| self.g.degree());
        PyArray1::from_vec(py, d) // moves the Vec into numpy: no copy
    }
    /// Zero-copy read-only views of the memory-mapped CSR arrays.
    fn offsets_view<'py>(slf: &Bound<'py, Self>) -> Bound<'py, PyArray1<u64>> {
        let s = slf.get().g.offsets.as_slice();
        view_ro(slf.py(), s, slf.clone().into_any())
    }
    fn targets_view<'py>(slf: &Bound<'py, Self>) -> Bound<'py, PyArray1<u32>> {
        let s = slf.get().g.targets.as_slice();
        view_ro(slf.py(), s, slf.clone().into_any())
    }
    /// Zero-copy read-only numpy view of the adjacency of v.
    fn neighbors<'py>(slf: &Bound<'py, Self>, v: usize) -> PyResult<Bound<'py, PyArray1<u32>>> {
        let g = &slf.get().g;
        if v >= g.n() { return Err(PyIndexError::new_err("node out of range")); }
        let s = g.neighbors(v);
        Ok(view_ro(slf.py(), s, slf.clone().into_any()))
    }
    /// Python list copy of the adjacency (for comparison).
    fn neighbors_list<'py>(&self, py: Python<'py>, v: usize) -> PyResult<Bound<'py, PyList>> {
        if v >= self.g.n() { return Err(PyIndexError::new_err("node out of range")); }
        PyList::new(py, self.g.neighbors(v))
    }
    fn degree_of(&self, v: usize) -> PyResult<usize> {
        if v >= self.g.n() { return Err(PyIndexError::new_err("node out of range")); }
        Ok(self.g.neighbors(v).len())
    }
    /// Batched: neighbors of many nodes, returned as (offsets, targets) numpy arrays.
    fn neighbors_batch<'py>(&self, py: Python<'py>, vs: PyReadonlyArray1<u32>) -> PyResult<(Bound<'py, PyArray1<u64>>, Bound<'py, PyArray1<u32>>)> {
        let vs = vs.as_slice()?;
        let n = self.g.n();
        if vs.iter().any(|&v| v as usize >= n) { return Err(PyIndexError::new_err("node out of range")); }
        let (o, t) = py.detach(|| {
            let mut o = Vec::with_capacity(vs.len() + 1); o.push(0u64);
            let mut t = Vec::new();
            for &v in vs { t.extend_from_slice(self.g.neighbors(v as usize)); o.push(t.len() as u64); }
            (o, t)
        });
        Ok((PyArray1::from_vec(py, o), PyArray1::from_vec(py, t)))
    }
    /// Parallel PageRank (NetworkX semantics). Releases the GIL. Returns numpy f64 (zero-copy move).
    #[pyo3(signature = (alpha=0.85, max_iter=100, tol=1e-6, threads=None))]
    fn pagerank<'py>(&self, py: Python<'py>, alpha: f64, max_iter: usize, tol: f64, threads: Option<usize>) -> (Bound<'py, PyArray1<f64>>, usize, usize) {
        let g = self.g.clone();
        let (x, it) = py.detach(move || with_threads(threads, || g.pagerank(alpha, max_iter, tol)));
        let ptr = x.as_ptr() as usize;
        (PyArray1::from_vec(py, x), it, ptr)
    }
    /// Same, returned through the Arrow PyCapsule interface.
    #[pyo3(signature = (alpha=0.85, max_iter=100, tol=1e-6, threads=None))]
    fn pagerank_arrow(&self, py: Python<'_>, alpha: f64, max_iter: usize, tol: f64, threads: Option<usize>) -> (ArrowArray, usize) {
        let g = self.g.clone();
        let (x, it) = py.detach(move || with_threads(threads, || g.pagerank(alpha, max_iter, tol)));
        let (arr, field) = f64_arrow(x, "pagerank");
        (ArrowArray { arr, field }, it)
    }
    /// PageRank as an Arrow struct array {node: u32, pagerank: f64} (== a RecordBatch over the C Data Interface).
    #[pyo3(signature = (alpha=0.85, max_iter=100, tol=1e-6, threads=None))]
    fn pagerank_table(&self, py: Python<'_>, alpha: f64, max_iter: usize, tol: f64, threads: Option<usize>) -> ArrowArray {
        let g = self.g.clone();
        let (x, ids) = py.detach(move || with_threads(threads, || {
            let (x, _) = g.pagerank(alpha, max_iter, tol);
            (x, (0..g.n() as u32).collect::<Vec<u32>>())
        }));
        let (a1, f1) = u32_arrow(ids, "node");
        let (a2, f2) = f64_arrow(x, "pagerank");
        let s = StructArray::new(vec![f1.clone(), f2.clone()].into(), vec![a1, a2], None);
        let field = Field::new("", s.data_type().clone(), false);
        ArrowArray { arr: Arc::new(s), field }
    }
    /// Connected components (parallel union-find); label = min node id in component.
    #[pyo3(signature = (threads=None))]
    fn wcc<'py>(&self, py: Python<'py>, threads: Option<usize>) -> Bound<'py, PyArray1<u32>> {
        let g = self.g.clone();
        let l = py.detach(move || with_threads(threads, || g.wcc()));
        PyArray1::from_vec(py, l)
    }
    /// Pure-Rust no-op round trip, to measure raw call overhead.
    fn noop(&self, v: usize) -> usize { v }
}

fn view_ro<'py, T: numpy::Element>(py: Python<'py>, s: &[T], owner: Bound<'py, PyAny>) -> Bound<'py, PyArray1<T>> {
    use numpy::npyffi::{get_type_object, npy_intp, NpyTypes, PY_ARRAY_API};
    use numpy::PyArrayDescrMethods;
    let mut dims = [s.len() as npy_intp];
    unsafe {
        let ptr = PY_ARRAY_API.PyArray_NewFromDescr(py, get_type_object(py, NpyTypes::PyArray_Type),
            T::get_dtype(py).into_dtype_ptr(), 1, dims.as_mut_ptr(), std::ptr::null_mut(),
            s.as_ptr() as *mut std::ffi::c_void, 0 /* not writeable */, std::ptr::null_mut());
        PY_ARRAY_API.PyArray_SetBaseObject(py, ptr as *mut numpy::npyffi::PyArrayObject, owner.into_ptr());
        Bound::from_owned_ptr(py, ptr).cast_into_unchecked()
    }
}

#[pymodule]
fn _glab(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Graph>()?;
    m.add_class::<ArrowArray>()?;
    m.add_function(wrap_pyfunction!(num_threads, m)?)?;
    Ok(())
}

#[pyfunction]
fn num_threads() -> usize { rayon::current_num_threads() }
