// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph compact FILE [--out NEW]` (`docs/api-sketch.md` § 5.1; ADR 0027;
//! `design/DECISIONS.md`, "The `compact` command"): a copy of the graph file that holds
//! only what its newest commit uses.
//!
//! In place, the copy is written as `FILE.partial` and renamed over `FILE` once
//! durable; a `FileSink` on `FILE` is held from before the copy starts until the
//! rename is durable, so no other writer commits to the file that is being replaced,
//! and a stop at any moment leaves `FILE` as it was (C-1). With `--out NEW`, `FILE` is
//! not changed.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use everygraph_core::ByteSource;
use everygraph_format::container::{ContainerError, compact};
use everygraph_io::FileSink;
use serde_json::json;

use crate::args::{Common, Progress};
use crate::failure::{self, Failure};
use crate::graph;
use crate::ingest::is_graph;
use crate::progress::{self, Lines};
use crate::render::{Style, counted, size};
use crate::table::Partial;
use everygraph_ingest::native;

const HELP: &str = "\
Write a copy of a graph file that holds only what its newest commit uses, and so
reclaim the bytes of replaced results.

Usage
  everygraph compact FILE [--out NEW] [OPTIONS]

Options
  --out NEW             write the copy to NEW and leave FILE as it is; without it,
                        the copy replaces FILE once it is complete
  --overwrite           replace NEW if it is in the way
  --json                print the file document, with nothing around it
  --progress plain      progress as one line per finished phase, never redrawn
  -h, --help            this text

Short form: -o for --out.
";

/// What the command was asked.
#[derive(Debug, clap::Args)]
#[command(override_help = HELP)]
pub(crate) struct Args {
    #[arg(value_name = "FILE")]
    file: PathBuf,
    #[arg(long, short = 'o', value_name = "NEW")]
    out: Option<PathBuf>,
    #[arg(long)]
    overwrite: bool,
    #[arg(long)]
    json: bool,
    #[arg(long, value_enum, default_value_t)]
    progress: Progress,
}

/// Runs the command; returns the exit status.
pub(crate) fn main(args: &Args) -> u8 {
    let common = Common {
        out: args.out.clone(),
        overwrite: args.overwrite,
        json: args.json,
        progress: args.progress,
        ..Common::default()
    };
    run(&args.file, &common).unwrap_or_else(|failure| failure.report(args.json))
}

fn run(path: &Path, common: &Common) -> Result<u8, Failure> {
    let started = Instant::now();
    // `--out FILE` is in place.
    let out = common.out.as_ref().filter(|out| {
        let real = |p: &Path| std::fs::canonicalize(p).ok();
        real(out).is_none() || real(out) != real(path)
    });
    if let Some(out) = out {
        if out.exists() && !common.overwrite {
            return Err(failure::output_exists(out, is_graph(out)));
        }
    }
    let busy = |e: io::Error| {
        if e.kind() == io::ErrorKind::ResourceBusy {
            Failure::new(
                "EverygraphError",
                format!("Another program is writing {}.", path.display()),
                failure::FAILED,
            )
            .ways(vec![(
                "run the same command again once it has finished".into(),
                String::new(),
            )])
            .on_existing_file()
        } else {
            failure::io("write", path, &e).on_existing_file()
        }
    };
    // Held until the copy has replaced the file: no other writer commits meanwhile.
    let held = match out {
        None => Some(FileSink::open(path).map_err(busy)?),
        Some(_) => None,
    };
    let file = graph::container(path)?;
    let destination = out.cloned().unwrap_or_else(|| path.to_path_buf());
    let partial = PathBuf::from(format!("{}.partial", destination.display()));
    let sink = FileSink::create(&partial).map_err(|e| failure::io("write", &partial, &e))?;
    let mut guard = Partial(Some(partial.clone()));
    let lines = Lines::new(Style::stderr().progress(common.plain()));
    lines.start("copying");
    let sink = compact(&file, sink).map_err(|e| match e {
        ContainerError::Write(e) => failure::io("write", &partial, &io::Error::other(e)),
        e => graph::container_error(path, &e),
    })?;
    let bytes = sink.len();
    drop(sink);
    let before = file.source().len();
    let sections = file.entries().len() as u64;
    drop(file);
    everygraph_io::persist(&partial, &destination)
        .map_err(|e| failure::io("write", &destination, &e))?;
    guard.0 = None;
    drop(held);
    let reclaimed = before.saturating_sub(bytes);
    lines.done_if_long("copying", &counted(sections, "section", "sections"));

    let seconds = started.elapsed().as_secs_f64();
    let facts = [size(bytes), format!("{} reclaimed", size(reclaimed))];
    progress::made(&destination, &facts, seconds);
    if common.json {
        let job = native::job_id(&[
            "compact",
            &native::canonical(path).display().to_string(),
            &native::canonical(&destination).display().to_string(),
        ]);
        let doc = json!({
            "schema": 1,
            "job": job,
            "operation": "compact",
            "made": {"path": destination.display().to_string(), "kind": "graph", "bytes": bytes},
            "counts": {"sections": sections, "reclaimed_bytes": reclaimed},
            "dropped": {},
            "kept": [],
            "elapsed_seconds": seconds,
        });
        println!("{doc}");
    }
    Ok(0)
}
