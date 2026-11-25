// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `everygraph-bench` command. Today it manages datasets (issue #5); the benchmark
//! harness (issue #4) adds its commands next to these.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line program talks to its user on the standard streams"
)]

use std::path::PathBuf;
use std::process::ExitCode;

use everygraph_bench::datasets::{
    self, DataRoot, FetchOptions, FileStatus, Outcome, Registry, human_bytes, markdown,
};

const USAGE: &str = "\
usage: everygraph-bench [--registry FILE] [--data DIR] <command> [options]

commands:
  fetch <name|tier1|tier2|tier3|all>...   download (or generate) and verify datasets
      --connections N      parallel connections per file (default 8)
      --from-dir DIR       copy files of the same name from DIR if they match
      --allow-unpinned     also fetch files the registry pins no checksum for
      --replace            delete placed files that no longer match, fetch again
  verify <name|tier1|tier2|tier3|all>...  re-read present files and check them
  list [--markdown]                       show datasets and what is on disk
      --markdown           print the tables of datasets/README.md instead
      --write-readme FILE  replace the generated part of FILE with them
  path <name>                             print the path of each file of a dataset

tierN includes the lower tiers. The data root is --data, else $EVERYGRAPH_DATA,
else ./data. The registry is the datasets/datasets.toml built into this binary
unless --registry names another.";

/// `println!` that ignores a closed standard output (`everygraph-bench list | head`)
/// instead of panicking.
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ignored = writeln!(std::io::stdout(), $($arg)*);
    }};
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(Failure::Usage(msg)) => {
            eprintln!("everygraph-bench: {msg}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Failure::Error(msg)) => {
            eprintln!("everygraph-bench: {msg}");
            ExitCode::from(1)
        }
    }
}

enum Failure {
    Usage(String),
    Error(String),
}

impl From<datasets::Error> for Failure {
    fn from(e: datasets::Error) -> Self {
        Failure::Error(e.to_string())
    }
}

fn usage(msg: impl Into<String>) -> Failure {
    Failure::Usage(msg.into())
}

/// Splits `--flag value` options from positional arguments.
struct Args {
    positional: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

const VALUE_FLAGS: &[&str] = &[
    "--registry",
    "--data",
    "--connections",
    "--from-dir",
    "--write-readme",
];
const BOOL_FLAGS: &[&str] = &[
    "--allow-unpinned",
    "--replace",
    "--markdown",
    "--help",
    "-h",
];

impl Args {
    fn parse(raw: &[String]) -> Result<Self, Failure> {
        let mut positional = Vec::new();
        let mut flags = Vec::new();
        let mut it = raw.iter();
        while let Some(a) = it.next() {
            if let Some((name, value)) = a.split_once('=').filter(|(n, _)| n.starts_with("--")) {
                if !VALUE_FLAGS.contains(&name) {
                    return Err(usage(format!("unknown option {name}")));
                }
                flags.push((name.to_string(), Some(value.to_string())));
            } else if VALUE_FLAGS.contains(&a.as_str()) {
                let value = it
                    .next()
                    .ok_or_else(|| usage(format!("{a} needs a value")))?;
                flags.push((a.clone(), Some(value.clone())));
            } else if BOOL_FLAGS.contains(&a.as_str()) {
                flags.push((a.clone(), None));
            } else if a.starts_with('-') {
                return Err(usage(format!("unknown option {a}")));
            } else {
                positional.push(a.clone());
            }
        }
        Ok(Self { positional, flags })
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }

    fn only(&self, allowed: &[&str]) -> Result<(), Failure> {
        for (n, _) in &self.flags {
            if !allowed.contains(&n.as_str()) && n != "--registry" && n != "--data" {
                return Err(usage(format!("{n} does not apply to this command")));
            }
        }
        Ok(())
    }
}

fn run(raw: &[String]) -> Result<ExitCode, Failure> {
    let args = Args::parse(raw)?;
    if args.has("--help") || args.has("-h") {
        out!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    let registry = match args.value("--registry") {
        Some(path) => Registry::load(&PathBuf::from(path))?,
        None => Registry::embedded()?,
    };
    // Every value flag must have a non-empty value: `--data=` would otherwise make the
    // working directory the data root.
    if let Some((name, _)) = args
        .flags
        .iter()
        .find(|(_, v)| v.as_deref().is_some_and(str::is_empty))
    {
        return Err(usage(format!("{name} needs a non-empty value")));
    }
    let root = args
        .value("--data")
        .map_or_else(DataRoot::from_env, DataRoot::new);
    let Some((command, operands)) = args.positional.split_first() else {
        return Err(usage("no command given"));
    };
    match command.as_str() {
        "fetch" => {
            args.only(&[
                "--connections",
                "--from-dir",
                "--allow-unpinned",
                "--replace",
            ])?;
            fetch(&registry, &root, operands, &args)
        }
        "verify" => {
            args.only(&[])?;
            verify(&registry, &root, operands)
        }
        "list" => {
            args.only(&["--markdown", "--write-readme"])?;
            if !operands.is_empty() {
                return Err(usage("list takes no operands"));
            }
            list(&registry, &root, &args)
        }
        "path" => {
            args.only(&[])?;
            let [name] = operands else {
                return Err(usage("path takes one dataset name"));
            };
            let ds = registry
                .get(name)
                .ok_or_else(|| Failure::Error(format!("no dataset named {name:?}")))?;
            for f in &ds.files {
                out!("{}", root.file_path(ds, f).display());
            }
            Ok(ExitCode::SUCCESS)
        }
        other => Err(usage(format!("unknown command {other:?}"))),
    }
}

fn selection<'r>(
    registry: &'r Registry,
    operands: &[String],
) -> Result<Vec<&'r datasets::Dataset>, Failure> {
    if operands.is_empty() {
        return Err(usage("name a dataset, tier1, tier2, tier3 or all"));
    }
    let mut out: Vec<&datasets::Dataset> = Vec::new();
    for op in operands {
        for ds in registry.select(op)? {
            if !out.iter().any(|d| d.name == ds.name) {
                out.push(ds);
            }
        }
    }
    Ok(out)
}

fn fetch(
    registry: &Registry,
    root: &DataRoot,
    operands: &[String],
    args: &Args,
) -> Result<ExitCode, Failure> {
    let selected = selection(registry, operands)?;
    let mut options = FetchOptions {
        allow_unpinned: args.has("--allow-unpinned"),
        replace: args.has("--replace"),
        from_dir: args.value("--from-dir").map(PathBuf::from),
        ..FetchOptions::default()
    };
    if let Some(n) = args.value("--connections") {
        options.download.connections = n
            .parse()
            .ok()
            .filter(|n| (1..=64).contains(n))
            .ok_or_else(|| usage("--connections takes a number from 1 to 64"))?;
    }
    let agent = datasets::agent(&options.download)?;
    let mut stderr = std::io::stderr();
    let mut failed = 0;
    eprintln!("data root: {}", root.path().display());
    for ds in &selected {
        let size = ds
            .total_size()
            .map_or_else(|| "size unknown".to_string(), human_bytes);
        eprintln!("{} (tier {}, {size})", ds.name, ds.tier);
        match datasets::fetch(root, &agent, ds, &options, &mut stderr) {
            Ok(outcomes) => {
                for (f, o) in ds.files.iter().zip(&outcomes) {
                    let what = match o {
                        Outcome::AlreadyVerified => "already verified".to_string(),
                        Outcome::VerifiedInPlace => "verified in place".to_string(),
                        Outcome::Placed { from } => format!("verified, from {from}"),
                        Outcome::PlacedUnpinned { from, sha256 } => format!(
                            "placed UNPINNED from {from}; sha256 = \"{sha256}\" (add it to the registry)"
                        ),
                    };
                    eprintln!("  {}: {what}", f.name);
                }
            }
            Err(e) => {
                eprintln!("  FAILED: {e}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        return Err(Failure::Error(format!(
            "{failed} of {} datasets failed",
            selected.len()
        )));
    }
    Ok(ExitCode::SUCCESS)
}

fn verify(registry: &Registry, root: &DataRoot, operands: &[String]) -> Result<ExitCode, Failure> {
    let selected = selection(registry, operands)?;
    let mut failed = 0;
    for ds in &selected {
        for f in &ds.files {
            match datasets::verify_file(root, ds, f) {
                Ok(status) => out!("{}/{}: {}", ds.name, f.name, status.label()),
                Err(e) => {
                    out!("{}/{}: FAILED: {e}", ds.name, f.name);
                    failed += 1;
                }
            }
        }
    }
    if failed > 0 {
        return Err(Failure::Error(format!(
            "{failed} files failed verification"
        )));
    }
    Ok(ExitCode::SUCCESS)
}

fn list(registry: &Registry, root: &DataRoot, args: &Args) -> Result<ExitCode, Failure> {
    if let Some(path) = args.value("--write-readme") {
        let path = PathBuf::from(path);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| Failure::Error(format!("reading {}: {e}", path.display())))?;
        let updated = markdown::splice(&text, &markdown::render(registry)).ok_or_else(|| {
            Failure::Error(format!(
                "{} has no generated-section markers",
                path.display()
            ))
        })?;
        std::fs::write(&path, updated)
            .map_err(|e| Failure::Error(format!("writing {}: {e}", path.display())))?;
        return Ok(ExitCode::SUCCESS);
    }
    if args.has("--markdown") {
        out!("{}", markdown::render(registry).trim_end());
        return Ok(ExitCode::SUCCESS);
    }
    out!("data root: {}", root.path().display());
    out!(
        "{:<32} {:>4}  {:<16} {:>10}  status",
        "name",
        "tier",
        "family",
        "download"
    );
    for ds in &registry.datasets {
        let statuses: Vec<FileStatus> = ds.files.iter().map(|f| root.status(ds, f)).collect();
        let status = summarize(&statuses);
        let size = if ds.is_generated() {
            "generated".to_string()
        } else {
            ds.total_size().map_or_else(|| "?".to_string(), human_bytes)
        };
        out!(
            "{:<32} {:>4}  {:<16} {:>10}  {status}",
            ds.name,
            ds.tier,
            ds.family.as_str(),
            size
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn summarize(statuses: &[FileStatus]) -> String {
    if statuses.iter().all(|s| *s == FileStatus::Verified) {
        return "verified".into();
    }
    if statuses.iter().all(|s| *s == FileStatus::Missing) {
        return "missing".into();
    }
    if let [one] = statuses {
        return one.label();
    }
    let parts: Vec<String> = statuses.iter().map(FileStatus::label).collect();
    parts.join(" / ")
}
