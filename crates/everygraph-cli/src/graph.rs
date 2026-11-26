// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Opening a graph file for the commands that read one: the committed part is mapped
//! by range (`docs/spec/eg-container.md` § 11), so that a result committed meanwhile,
//! by this process or another, never touches what is mapped.

use std::fs::File;
use std::path::Path;

use everygraph_core::{ByteSource, Memory, ReadError};
use everygraph_format::container::{Container, ContainerError};
use everygraph_ingest::build::BuildError;
use everygraph_ingest::graph::Graph;
use everygraph_io::{FileSource, MmapSource};

use crate::failure::{self, Failure};
use crate::render::{Style, counted, size};

/// The container of the graph file at `path`, on a map of its committed part.
///
/// # Errors
///
/// `IoError` when the file cannot be read; `CorruptFile` when it is not a graph file.
pub(crate) fn container(path: &Path) -> Result<Container<MmapSource>, Failure> {
    let io = |e: &std::io::Error| failure::io("read", path, e).on_existing_file();
    if path.is_dir() {
        let e = std::io::Error::new(std::io::ErrorKind::IsADirectory, "it is a directory");
        return Err(io(&e));
    }
    let file = File::open(path).map_err(|e| io(&e))?;
    let probe = file
        .try_clone()
        .and_then(FileSource::from_file)
        .map_err(|e| io(&e))?;
    let probe = Container::open(probe).map_err(|e| container_error(path, &e))?;
    let map = MmapSource::from_file_range(&file, probe.stable_range()).map_err(|e| io(&e))?;
    probe
        .with_source(map)
        .map_err(|e| container_error(path, &e))
}

/// The graph file at `path`, its index held under `memory`.
///
/// # Errors
///
/// As [`container`]; `CorruptFile` when the graph's sections are not valid;
/// `InsufficientBudget` when its index does not fit.
pub(crate) fn open(path: &Path, memory: &Memory, budget: u64) -> Result<Graph<MmapSource>, Failure> {
    Graph::open(container(path)?, memory).map_err(|e| match e {
        BuildError::Container(e) => container_error(path, &e),
        BuildError::Budget(e) => failure::memory("opening of the graph", budget, None, &e)
            .on_existing_file(),
        other => failure::corrupt(path, &other),
    })
}

/// The failure for a container that cannot be opened or read.
pub(crate) fn container_error(path: &Path, e: &ContainerError) -> Failure {
    match e {
        ContainerError::Read(ReadError::Backend(e)) => Failure::new(
            "IoError",
            format!("Cannot read {}: {e}.", path.display()),
            failure::FAILED,
        )
        .key("path", path.display().to_string())
        .on_existing_file(),
        ContainerError::NotAContainer | ContainerError::NotCommitted => Failure::new(
            "CorruptFile",
            format!("{} is not an Everygraph graph file.", path.display()),
            failure::DAMAGED,
        )
        .ways(vec![(
            format!("everygraph ingest FILE --out {}", path.display()),
            "build a graph from a file of edges".into(),
        )])
        .on_existing_file(),
        ContainerError::NoSuchSection(_) => Failure::new(
            "CorruptFile",
            format!("{} holds no graph.", path.display()),
            failure::DAMAGED,
        )
        .ways(vec![(
            format!("everygraph verify {}", path.display()),
            "what the file holds".into(),
        )])
        .on_existing_file(),
        other => failure::corrupt(path, other),
    }
}

/// The headline of an operation on a graph (§ 3.4): name · size · nodes · edges.
pub(crate) fn headline<S: ByteSource>(path: &Path, graph: &Graph<S>, style: Style) -> String {
    let dot = style.dot();
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    format!(
        "{}{dot}{}{dot}{}{dot}{}",
        style.bold(&name),
        size(graph.file().committed_len()),
        counted(u64::from(graph.nodes()), "node", "nodes"),
        counted(graph.edges(), "edge", "edges")
    )
}
