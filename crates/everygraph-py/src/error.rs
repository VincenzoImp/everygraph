// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Errors of the engine, as `NativeError(class, fields)`: the class of
//! `docs/api-sketch.md` § 2.7 and the facts its message is made from. The Python
//! package writes the message (`everygraph/_errors.py`). Errors of the operating system
//! are raised as Python's own `OSError` subclasses.

use std::path::Path;

use everygraph_core::{InsufficientBudget, MemoryError};
use everygraph_ingest::build::BuildError;
use everygraph_ingest::graph::OpenError;
use everygraph_ingest::ids::DictionaryError;
use everygraph_ingest::native::WriteError;
use everygraph_ingest::text::TextError;
use everygraph_kernels::edge_map::Error as KernelError;
use pyo3::exceptions::{PyException, PyOSError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

pyo3::create_exception!(
    _native,
    NativeError,
    PyException,
    "An engine error: (class, fields)."
);

/// One field of an error.
pub(crate) enum Value {
    Text(String),
    Count(u64),
    /// Invalid rows: file, line, problem, column, excerpt (the `problems` of
    /// `docs/ux-contract.md` § 6.1).
    Problems(Vec<(String, u64, String, Option<String>, String)>),
}

/// An error class and its facts.
pub(crate) struct Failure {
    class: &'static str,
    fields: Vec<(&'static str, Value)>,
}

impl Failure {
    pub(crate) fn new(class: &'static str) -> Self {
        Self {
            class,
            fields: Vec::new(),
        }
    }

    pub(crate) fn with(mut self, key: &'static str, value: Value) -> Self {
        self.fields.push((key, value));
        self
    }

    pub(crate) fn text(self, key: &'static str, value: impl ToString) -> Self {
        self.with(key, Value::Text(value.to_string()))
    }

    pub(crate) fn count(self, key: &'static str, value: u64) -> Self {
        self.with(key, Value::Count(value))
    }

    /// The Python exception.
    pub(crate) fn into_err(self, py: Python<'_>) -> PyErr {
        let class = self.class;
        match self.fields(py) {
            Ok(fields) => NativeError::new_err((class, fields.unbind())),
            Err(e) => e,
        }
    }

    fn fields(self, py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
        let fields = PyDict::new(py);
        for (key, value) in self.fields {
            match value {
                Value::Text(t) => fields.set_item(key, t)?,
                Value::Count(c) => fields.set_item(key, c)?,
                Value::Problems(rows) => {
                    let rows = rows
                        .into_iter()
                        .map(|(file, line, problem, column, excerpt)| {
                            let row = PyDict::new(py);
                            row.set_item("file", file)?;
                            row.set_item("line", line)?;
                            row.set_item("problem", problem)?;
                            row.set_item("column", column)?;
                            row.set_item("excerpt", excerpt)?;
                            Ok(row)
                        });
                    fields.set_item(key, rows.collect::<PyResult<Vec<_>>>()?)?;
                }
            }
        }
        Ok(fields)
    }
}

/// What a planned operation can fail with: an engine error, or one of the system.
pub(crate) enum Fail {
    Engine(Failure),
    Os(std::io::Error, String),
}

impl Fail {
    pub(crate) fn into_err(self, py: Python<'_>) -> PyErr {
        match self {
            Self::Engine(f) => f.into_err(py),
            Self::Os(e, path) => {
                let err = PyErr::from(e);
                let message = format!("{path}: {}", err.value(py));
                match err.get_type(py).call1((message,)) {
                    Ok(value) => PyErr::from_value(value),
                    Err(_) => PyOSError::new_err(path),
                }
            }
        }
    }
}

impl From<Failure> for Fail {
    fn from(f: Failure) -> Self {
        Self::Engine(f)
    }
}

/// A step that the budget `budget` cannot hold.
pub(crate) fn budget(refused: &InsufficientBudget, budget: u64) -> Failure {
    Failure::new("InsufficientBudget")
        .text("step", refused.purpose)
        .count("needs_bytes", refused.needed)
        .count("available_bytes", refused.available)
        .count("budget_bytes", budget)
}

fn memory(e: MemoryError, budget_bytes: u64) -> Failure {
    match e {
        MemoryError::Budget(refused) => budget(&refused, budget_bytes),
        other => Failure::new("EverygraphError").text("message", other),
    }
}

/// A failure of the text reader on `input`.
pub(crate) fn text(input: &Path, e: TextError) -> Fail {
    let file = input.display().to_string();
    match e {
        TextError::Read(cause) => Fail::Os(std::io::Error::other(cause.to_string()), file),
        TextError::InvalidRows(summary) => {
            let problems = summary
                .problems
                .iter()
                .map(|p| {
                    let excerpt = String::from_utf8_lossy(&p.excerpt).into_owned();
                    (
                        file.clone(),
                        p.line,
                        p.to_string(),
                        p.column.clone(),
                        excerpt,
                    )
                })
                .collect();
            Failure::new("InvalidInput")
                .text("file", file)
                .with("problems", Value::Problems(problems))
                .count("invalid_rows", summary.invalid_rows)
                .count("rows_checked", summary.lines)
                .into()
        }
        other @ (TextError::UnknownColumn { .. }
        | TextError::NoHeader { .. }
        | TextError::BeyondHeader { .. }) => Failure::new("ValueError")
            .text("message", format!("Cannot read {file}: {other}."))
            .into(),
        other => Failure::new("InvalidInput")
            .text("file", file)
            .text("reason", other)
            .into(),
    }
}

/// A failure of the build of `input` under `budget`.
pub(crate) fn write(input: &Path, budget_bytes: u64, e: WriteError) -> Fail {
    match e {
        WriteError::Io { path, error } => Fail::Os(error, path.display().to_string()),
        WriteError::Build(e) => build(input, budget_bytes, e),
        WriteError::Withdrawn => Failure::new("EverygraphError")
            .text("message", "Stopped.")
            .into(),
    }
}

fn build(input: &Path, budget_bytes: u64, e: BuildError) -> Fail {
    match e {
        BuildError::Text(e) => text(input, e),
        BuildError::Budget(refused) => budget(&refused, budget_bytes).into(),
        BuildError::Memory(e) => memory(e, budget_bytes).into(),
        BuildError::InputChanged => Failure::new("InvalidInput")
            .text("file", input.display())
            .text("reason", "it changed while it was read")
            .into(),
        BuildError::InvalidKeys(keys) => Failure::new("InvalidInput")
            .text("file", input.display())
            .text("reason", "some identifiers do not fit their column")
            .text(
                "examples",
                keys.first
                    .iter()
                    .map(|k| k.key.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
            .count("invalid_rows", keys.count)
            .into(),
        BuildError::Scratch(e) => Fail::Os(e, "the scratch files".into()),
        BuildError::ColumnClash(name) => Failure::new("InvalidInput")
            .text("file", input.display())
            .text(
                "reason",
                format!("two stored columns would be named {name}"),
            )
            .into(),
        BuildError::Format(e) => Failure::new("CorruptFile").text("why", e).into(),
        BuildError::Container(e) => Failure::new("CorruptFile").text("why", e).into(),
        other => Failure::new("EverygraphError")
            .text("message", format!("The build failed: {other}."))
            .into(),
    }
}

/// A graph file that cannot be opened.
pub(crate) fn open(path: &Path, e: OpenError, budget_bytes: u64) -> Fail {
    match e {
        OpenError::Read(e) => Fail::Os(
            std::io::Error::other(e.to_string()),
            path.display().to_string(),
        ),
        OpenError::Budget(refused) => budget(&refused, budget_bytes).into(),
        OpenError::Corrupt { section, what } => {
            let f = Failure::new("CorruptFile")
                .text("path", path.display())
                .text("why", what);
            match section {
                Some(s) => f.text("section", s).into(),
                None => f.into(),
            }
        }
        other => Failure::new("CorruptFile")
            .text("path", path.display())
            .text("why", other)
            .into(),
    }
}

/// A dictionary that fails.
pub(crate) fn dictionary(e: DictionaryError, budget_bytes: u64) -> Failure {
    match e {
        DictionaryError::Budget(refused) => budget(&refused, budget_bytes),
        other => Failure::new("CorruptFile")
            .text("section", "ids")
            .text("why", other),
    }
}

/// A kernel that fails, in `operation`.
pub(crate) fn kernel(operation: &str, e: KernelError, budget_bytes: u64) -> Failure {
    match e {
        KernelError::Memory(e) => memory(e, budget_bytes),
        e @ (KernelError::Decode(_) | KernelError::Format(_) | KernelError::Target { .. }) => {
            Failure::new("CorruptFile")
                .text("section", "adjacency.out")
                .text("why", e)
        }
        other => Failure::new("EverygraphError")
            .text("message", format!("{operation}() failed: {other}.")),
    }
}
