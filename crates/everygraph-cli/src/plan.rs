// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The plan of an ingest (`docs/ux-contract.md` § 3): what was understood of the input,
//! the machine, the budget, the regime and the estimates, as text (§ 3.4) and as the
//! plan document (§ 3.6). The estimates come from `everygraph_ingest::build::estimate`,
//! the rules the build applies.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use everygraph_core::ByteSource;
use everygraph_ingest::build::{self, DirectedFrom, Duplicates, Estimate, Regime, SelfLoops};
use everygraph_ingest::text::{Compression, IdType, Ids, TextError, TextReader};
use everygraph_io::FileSource;

use crate::failure::{self, Failure};
use crate::ingest::Args;
use crate::machine::{Disk, Machine, disk};
use crate::render::{Style, about, count, json_string, rough_count, rough_size, size, to_f64};

/// Text read per second and per thread, for the time estimate. A placeholder of the
/// cost model, not a measurement.
const MODEL_BYTES_PER_SECOND_PER_THREAD: f64 = 50e6;
/// A compressed edge list holds about this many times its size in text, in lines of
/// about this many bytes (placeholders of the cost model).
const MODEL_COMPRESSION_RATIO: u64 = 4;
const MODEL_LINE_BYTES: u64 = 15;
/// The share of the time estimate of each phase (placeholders of the cost model).
const PHASES: [(&str, f64); 4] = [
    ("reading", 0.3),
    ("identifiers", 0.05),
    ("building", 0.6),
    ("checking", 0.05),
];
/// An operation estimated to take at least this long prints its plan (P-3; placeholder).
const LONG_SECONDS: f64 = 5.0;

/// What the start of the input says about its rows.
struct Sample {
    build: build::Sample,
    exact: bool,
    /// Distinct identifiers in the first block.
    distinct: u64,
}

fn sample(reader: &TextReader<FileSource>) -> Result<Sample, TextError> {
    let bytes = reader.source().len();
    let block_bytes = u64::try_from(reader.block_bytes()).unwrap_or(u64::MAX);
    let Some(blocks) = reader.blocks()? else {
        let rows = bytes * MODEL_COMPRESSION_RATIO / MODEL_LINE_BYTES;
        let blocks = (bytes * MODEL_COMPRESSION_RATIO).div_ceil(block_bytes);
        return Ok(Sample {
            build: build::Sample::new(rows, block_bytes / MODEL_LINE_BYTES, block_bytes, blocks),
            exact: false,
            distinct: 0,
        });
    };
    if blocks == 0 {
        return Ok(Sample {
            build: build::Sample::new(0, 0, block_bytes, 0),
            exact: true,
            distinct: 0,
        });
    }
    let (batch, _) = reader.read_block(0)?;
    let rows = u64::try_from(batch.len()).unwrap_or(u64::MAX);
    let distinct = match (&batch.source, &batch.target) {
        (Ids::Integer(s), Ids::Integer(t)) => s.iter().chain(t).collect::<HashSet<_>>().len(),
        (Ids::String(s), Ids::String(t)) => s.iter().chain(t.iter()).collect::<HashSet<_>>().len(),
        _ => 0,
    };
    let blocks = u64::try_from(blocks).unwrap_or(u64::MAX);
    Ok(Sample {
        build: build::Sample::new(rows * blocks, rows, block_bytes, blocks),
        exact: blocks == 1,
        distinct: u64::try_from(distinct).unwrap_or(u64::MAX),
    })
}

/// Where an argument came from (`origin` of § 3.6).
fn origin(given: bool) -> &'static str {
    if given { "argument" } else { "default" }
}

/// The plan of one ingest.
pub(crate) struct Plan {
    pub(crate) budget: u64,
    pub(crate) threads: NonZeroUsize,
    pub(crate) estimate: Estimate,
    machine: Machine,
    sample: Sample,
    seconds: f64,
    directed: (bool, DirectedFrom),
    scratch_root: PathBuf,
    scratch: Option<Disk>,
    output: PathBuf,
    output_disk: Option<Disk>,
}

impl Plan {
    /// The plan of `args` on `reader`, writing `output`, with scratch in `scratch_root`.
    pub(crate) fn new(
        args: &Args,
        reader: &TextReader<FileSource>,
        directed: (bool, DirectedFrom),
        output: &Path,
        scratch_root: &Path,
    ) -> Result<Self, Failure> {
        let machine = Machine::probe();
        let budget = machine.budget(&args.common)?;
        let threads = machine.threads(&args.common);
        let sample = sample(reader).map_err(|e| failure::text(&args.input, e))?;
        let estimate = build::estimate(&sample.build, directed.0, threads, budget);
        let text_bytes = match reader.layout().compression {
            Compression::None => reader.source().len(),
            _ => reader.source().len() * MODEL_COMPRESSION_RATIO,
        };
        let seconds = to_f64(text_bytes)
            / (MODEL_BYTES_PER_SECOND_PER_THREAD
                * to_f64(u64::try_from(threads.get()).unwrap_or(u64::MAX)));
        Ok(Self {
            budget,
            threads,
            estimate,
            machine,
            sample,
            seconds,
            directed,
            scratch: disk(scratch_root),
            scratch_root: scratch_root.to_path_buf(),
            output_disk: output.parent().and_then(|p| {
                disk(if p.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    p
                })
            }),
            output: output.to_path_buf(),
        })
    }

    /// Whether the budget holds what the build needs before it can know the nodes.
    pub(crate) fn fits(&self) -> bool {
        self.budget >= self.estimate.smallest_budget
    }

    /// Whether the plan block is printed without being asked (P-3).
    pub(crate) fn long(&self) -> bool {
        self.seconds >= LONG_SECONDS
    }

    /// The machine's memory in all, if known.
    pub(crate) fn machine_memory(&self) -> Option<u64> {
        self.machine.total()
    }

    /// The refusal of a budget below the smallest.
    pub(crate) fn refusal(&self) -> Failure {
        failure::refusal(
            self.estimate.smallest_budget,
            self.budget,
            self.machine_memory(),
            self.threads.get(),
        )
    }

    fn ratio(&self) -> f64 {
        to_f64(self.estimate.working_bytes) / to_f64(self.budget.max(1))
    }

    fn reason(&self, times: &str) -> String {
        let ratio = self.ratio();
        match self.estimate.regime {
            Regime::InMemory => format!("the graph fits the memory budget ({ratio:.1}{times})"),
            Regime::OnDisk => format!("the graph is {ratio:.1}{times} the memory budget"),
        }
    }

    /// The headline, the summary and, with `block`, the plan block (§ 3.4).
    pub(crate) fn text(
        &self,
        args: &Args,
        reader: &TextReader<FileSource>,
        style: Style,
        block: bool,
    ) -> String {
        let dot = style.dot();
        let layout = reader.layout();
        let name = args
            .input
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let rows = self.sample.build.rows;
        let rows = if self.sample.exact {
            format!("{} rows", count(rows))
        } else {
            format!("~{} rows", rough_count(rows))
        };
        let mut out = format!(
            "{}{dot}{}{dot}{rows}\n\n",
            style.bold(&name),
            size(reader.source().len())
        );
        let ids = if layout.ids == IdType::String {
            "text"
        } else {
            "integers"
        };
        let _ = writeln!(out, "  {}       {ids}", style.dim("ids"));
        let direction = match self.directed {
            (true, DirectedFrom::ColumnNames) => "directed (from the column names)",
            (false, DirectedFrom::Default) => "undirected (the column names give no direction)",
            (true, _) => "directed",
            (false, _) => "undirected",
        };
        let (duplicates, loops) = (duplicates_text(args), loops_text(args));
        let value = format!("{direction}{dot}duplicates {duplicates}{dot}self-loops {loops}");
        if 12 + value.chars().count() <= 80 {
            let _ = writeln!(out, "  {}     {value}", style.dim("edges"));
        } else {
            // A second line, aligned with the first value, when one does not fit.
            let _ = writeln!(out, "  {}     {direction}", style.dim("edges"));
            let _ = writeln!(
                out,
                "            duplicates {duplicates}{dot}self-loops {loops}"
            );
        }
        let arrow = style.arrow();
        let _ = writeln!(
            out,
            "  {}   {} {arrow} source{dot}{} {arrow} target",
            style.dim("columns"),
            layout.column_name(layout.source),
            layout.column_name(layout.target)
        );
        if !block {
            return out;
        }
        out.push('\n');
        let memory = match self.machine.memory {
            Some(m) => format!(
                "{} RAM ({} free)",
                rough_size(m.machine_total),
                rough_size(m.available())
            ),
            None => "memory unknown".into(),
        };
        let disk = self
            .output_disk
            .as_ref()
            .map_or_else(String::new, |d| format!("{dot}{} free", rough_size(d.free)));
        let _ = writeln!(
            out,
            "  {}   {} cores{dot}{memory}{disk}",
            style.dim("machine"),
            self.machine.cores
        );
        let from = if args.common.memory.is_some() {
            "--memory"
        } else {
            "default: half of free RAM"
        };
        let _ = writeln!(
            out,
            "  {}    {} memory ({from})",
            style.dim("budget"),
            size(self.budget)
        );
        if let Some(free) = self.machine.memory.map(|m| m.available()) {
            if self.budget > free {
                let _ = writeln!(
                    out,
                    "            more than the {} free now: the system may swap",
                    rough_size(free)
                );
            }
        }
        let regime = match self.estimate.regime {
            Regime::InMemory => "build in memory",
            Regime::OnDisk => "build on disk",
        };
        let _ = writeln!(
            out,
            "  {}      {regime} {} {}",
            style.dim("plan"),
            style.dash(),
            self.reason(style.times())
        );
        let scratch = match self.estimate.scratch_bytes {
            0 => "no scratch".to_owned(),
            bytes => format!("{} scratch", rough_size(bytes)),
        };
        let _ = writeln!(
            out,
            "  {}  {}{dot}peak {}{dot}{scratch}{dot}result {}",
            style.dim("estimate"),
            about(self.seconds),
            rough_size(self.estimate.peak_bytes),
            rough_size(self.estimate.output_bytes)
        );
        out
    }

    /// The plan document (§ 3.6), with `refusal` when the plan does not fit.
    pub(crate) fn document(
        &self,
        args: &Args,
        reader: &TextReader<FileSource>,
        refusal: Option<&Failure>,
    ) -> String {
        let layout = reader.layout();
        let regime = match self.estimate.regime {
            Regime::InMemory => "in_memory",
            Regime::OnDisk => "on_disk",
        };
        let rows = self.sample.build.rows;
        let rows = if self.sample.exact {
            estimate_json(to_f64(rows), "exact")
        } else {
            estimate_json(to_f64(rows), "sampled")
        };
        let memory = self.machine.memory;
        let free = memory.map_or(0, |m| m.available());
        let source = layout.column_name(layout.source);
        let target = layout.column_name(layout.target);
        let compression = match layout.compression {
            Compression::None => "none",
            Compression::Gzip => "gzip",
            Compression::Zstd => "zstd",
        };
        let distinct = {
            let low = to_f64(self.sample.distinct);
            let high = to_f64(self.sample.build.rows.saturating_mul(2)).max(low);
            let value = (low * to_f64(self.sample.build.blocks)).clamp(low, high);
            format!("{{\"value\":{value},\"low\":{low},\"high\":{high},\"basis\":\"sampled\"}}")
        };
        let phases: Vec<String> = PHASES
            .iter()
            .map(|(name, share)| {
                format!(
                    "{{\"name\":\"{name}\",\"kept\":false,\"regime\":\"{regime}\",\"seconds\":{}}}",
                    estimate_json(self.seconds * share, "model")
                )
            })
            .collect();
        let ids_kind = if layout.ids == IdType::String {
            "text"
        } else {
            "integer"
        };
        let directed_from = match self.directed.1 {
            DirectedFrom::Caller => "caller",
            DirectedFrom::ColumnNames => "column names",
            DirectedFrom::Default => "default",
        };
        let (scratch_free, memory_backed) = self
            .scratch
            .as_ref()
            .map_or((0, false), |d| (d.free, d.memory_backed));
        let output_free = self
            .output_disk
            .as_ref()
            .map_or_else(String::new, |d| format!(",\"free_bytes\":{}", d.free));
        let mut out = String::new();
        let _ = write!(
            out,
            "{{\"schema\":1,\"operation\":\"ingest\",\
             \"parameters\":{{\"source\":{s},\"target\":{t},\"directed\":{dir},\
             \"self_loops\":\"{loops}\",\"duplicates\":\"{dups}\",\"ids\":\"{ids}\"}},\
             \"input\":{{\"path\":{path},\"bytes\":{bytes},\"format\":\"text\",\
             \"compression\":\"{compression}\",\"rows\":{rows}}},\
             \"graph\":null,\
             \"detected\":{{\"ids\":{{\"kind\":\"{ids_kind}\",\"distinct\":{distinct}}},\
             \"edges\":{{\"directed\":{dir},\"directed_from\":\"{directed_from}\",\
             \"duplicates\":\"{dup_text}\",\"self_loops\":\"{loop_text}\"}},\
             \"columns\":[{{\"name\":{s},\"role\":\"source\"}},{{\"name\":{t},\"role\":\"target\"}}]}},\
             \"machine\":{{\"cores\":{cores},\"hardware_threads\":{hw},\"memory_bytes\":{total},\
             \"memory_free_bytes\":{free},\"memory_limit_bytes\":{limit},\
             \"storage\":{{\"kind\":\"unknown\",\"free_bytes\":{output_free_bytes}}}}},\
             \"budget\":{{\"memory_bytes\":{budget},\"memory_from\":\"{memory_from}\",\
             \"above_free\":{above},\"threads\":{threads},\"threads_from\":\"{threads_from}\"}},\
             \"regime\":{{\"name\":\"{regime}\",\"reason\":{reason},\
             \"working_bytes\":{working},\"ratio\":{ratio}}},\
             \"phases\":[{phases}],\
             \"estimate\":{{\"seconds\":{seconds},\"peak_memory_bytes\":{peak},\
             \"scratch_bytes\":{scratch},\"output_bytes\":{output_bytes}}},\
             \"scratch\":{{\"path\":{scratch_path},\"from\":\"{scratch_from}\",\
             \"memory_backed\":{memory_backed},\"free_bytes\":{scratch_free}}},\
             \"output\":{{\"path\":{output}{output_free},\"kind\":\"graph\"}},\
             \"fits\":{fits},\"refusal\":{refusal}}}",
            s = json_string(&source),
            t = json_string(&target),
            dir = self.directed.0,
            loops = match args.self_loops {
                SelfLoops::Drop => "drop",
                SelfLoops::Keep => "keep",
            },
            dups = match args.duplicates {
                Duplicates::Events => "events",
                Duplicates::Collapse => "collapse",
                Duplicates::Sum => "sum",
            },
            ids = if args.ids == IdType::String {
                "string"
            } else {
                "auto"
            },
            path = json_string(&args.input.display().to_string()),
            bytes = reader.source().len(),
            dup_text = duplicates_text(args),
            loop_text = loops_text(args),
            cores = self.machine.cores,
            hw = self.machine.hardware_threads,
            total = memory.map_or(0, |m| m.machine_total),
            limit = memory
                .and_then(|m| m.limit)
                .map_or_else(|| "null".to_owned(), |l| l.to_string()),
            output_free_bytes = self.output_disk.as_ref().map_or(0, |d| d.free),
            budget = self.budget,
            memory_from = origin(args.common.memory.is_some()),
            above = memory.is_some_and(|_| self.budget > free),
            threads = self.threads,
            threads_from = origin(args.common.threads.is_some()),
            reason = json_string(&self.reason("×")),
            working = self.estimate.working_bytes,
            ratio = self.ratio(),
            phases = phases.join(","),
            seconds = estimate_json(self.seconds, "model"),
            peak = estimate_json(to_f64(self.estimate.peak_bytes), "model"),
            scratch = estimate_json(to_f64(self.estimate.scratch_bytes), "model"),
            output_bytes = estimate_json(to_f64(self.estimate.output_bytes), "model"),
            scratch_path = json_string(&self.scratch_root.display().to_string()),
            scratch_from = origin(args.temp_dir.is_some()),
            output = json_string(&self.output.display().to_string()),
            fits = self.fits(),
            refusal = refusal.map_or_else(|| "null".to_owned(), |r| r.document().to_string()),
        );
        out
    }
}

/// The `duplicates` value of `detected.edges`.
fn duplicates_text(args: &Args) -> &'static str {
    match args.duplicates {
        Duplicates::Events => "kept as events",
        Duplicates::Collapse => "collapsed",
        Duplicates::Sum => "summed",
    }
}

/// The `self_loops` value of `detected.edges`.
fn loops_text(args: &Args) -> &'static str {
    match args.self_loops {
        SelfLoops::Drop => "dropped",
        SelfLoops::Keep => "kept",
    }
}

/// An estimate triple (P-7): exact, or a model's value with a factor of two each way.
fn estimate_json(value: f64, basis: &str) -> String {
    let (low, high) = match basis {
        "exact" => (value, value),
        "sampled" => (value * 0.7, value * 1.3),
        _ => (value / 2.0, value * 2.0),
    };
    format!("{{\"value\":{value},\"low\":{low},\"high\":{high},\"basis\":\"{basis}\"}}")
}
