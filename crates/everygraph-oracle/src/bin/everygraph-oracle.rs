// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph-oracle`: writes the corpus, runs a kernel through the harness, shows which
//! checks catch the seeded bugs, and checks the golden files. See `oracle/README.md`.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line program talks to its user on the standard streams"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use everygraph_oracle::corpus::{self, Tier};
use everygraph_oracle::exchange;
use everygraph_oracle::external::External;
use everygraph_oracle::plugin::{
    self, Config, KernelUnderTest, Options, Prepared, Regime, RustReference, Storage,
};
use everygraph_oracle::toy::{Bug, Toy};

const USAGE: &str = "usage:
  everygraph-oracle corpus --tier ci|nightly --out DIR
  everygraph-oracle check --corpus DIR --plugin reference|toy|toy:BUG|external
        [--program PATH] [--arg A]... [--kernels k1,k2] [--directed] [--keeps-self-loops] [--weighted]
        [--threads 1,4] [--chunks 64,65536] [--regimes in_memory,mmap]
        [--require-python] [--max-findings N] [--show N]
  everygraph-oracle seeded-bugs [--corpus DIR]
  everygraph-oracle golden --dir DIR";

struct Args {
    rest: Vec<String>,
}

impl Args {
    fn value(&mut self, key: &str) -> Option<String> {
        let i = self.rest.iter().position(|a| a == key)?;
        if i + 1 >= self.rest.len() {
            return None;
        }
        let v = self.rest.remove(i + 1);
        self.rest.remove(i);
        Some(v)
    }

    fn values(&mut self, key: &str) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(v) = self.value(key) {
            out.push(v);
        }
        out
    }

    fn flag(&mut self, key: &str) -> bool {
        self.rest
            .iter()
            .position(|a| a == key)
            .map(|i| self.rest.remove(i))
            .is_some()
    }

    fn list<T: std::str::FromStr + Clone>(
        &mut self,
        key: &str,
        default: &[T],
    ) -> Result<Vec<T>, String> {
        self.value(key).map_or_else(
            || Ok(default.to_vec()),
            |v| {
                v.split(',')
                    .map(|x| x.parse().map_err(|_| format!("bad value {x} for {key}")))
                    .collect()
            },
        )
    }

    fn finish(&self) -> Result<(), String> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(format!("unexpected arguments: {:?}", self.rest))
        }
    }
}

fn main() -> ExitCode {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let cmd = argv.remove(0);
    let mut args = Args { rest: argv };
    let result = match cmd.as_str() {
        "corpus" => cmd_corpus(&mut args),
        "check" => cmd_check(&mut args),
        "seeded-bugs" => cmd_seeded_bugs(&mut args),
        "golden" => cmd_golden(&mut args),
        _ => Err(format!("unknown command {cmd}\n{USAGE}")),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("everygraph-oracle: {e}");
            ExitCode::from(2)
        }
    }
}

fn tier(s: &str) -> Result<Tier, String> {
    match s {
        "ci" => Ok(Tier::Ci),
        "nightly" => Ok(Tier::Nightly),
        _ => Err(format!("unknown tier {s}")),
    }
}

fn cmd_corpus(a: &mut Args) -> Result<bool, String> {
    let t = tier(&a.value("--tier").unwrap_or_else(|| "ci".into()))?;
    let out = PathBuf::from(a.value("--out").ok_or("--out is required")?);
    a.finish()?;
    let entries = corpus::entries(t);
    let (mut rows, mut kernels) = (0usize, 0usize);
    for e in &entries {
        exchange::write_entry(&out.join(&e.graph.name), e).map_err(|e| e.to_string())?;
        rows += e.graph.rows();
        kernels += e.kernels.len();
    }
    println!(
        "wrote {} graphs ({rows} rows, {kernels} graph-kernel pairs) to {}",
        entries.len(),
        out.display()
    );
    Ok(true)
}

fn load_corpus(dir: &Path) -> Result<Vec<Prepared>, String> {
    let dirs = exchange::list_corpus(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if dirs.is_empty() {
        return Err(format!("no graphs under {}", dir.display()));
    }
    dirs.iter()
        .map(|d| Prepared::load(d).map_err(|e| format!("{}: {e}", d.display())))
        .collect()
}

fn regime(s: &str) -> Result<Regime, String> {
    match s {
        "in_memory" => Ok(Regime::InMemory),
        "mmap" => Ok(Regime::Mmap),
        "stream" => Ok(Regime::Stream),
        _ => Err(format!("unknown regime {s}")),
    }
}

fn summarize(name: &str, report: &plugin::Report, show: usize, seconds: f64) {
    println!(
        "{name}: {} graphs, {} graph-kernel pairs checked ({} unsupported), {} runs, {} Python comparisons, {} pairs without a Python answer, {} findings, {seconds:.1} s",
        report.graphs,
        report.checked,
        report.unsupported,
        report.runs,
        report.python_comparisons,
        report.python_missing,
        report.findings.len()
    );
    for (check, count) in report.by_check() {
        println!("  {count:6}  {check}");
    }
    for f in report.findings.iter().take(show) {
        let mut line = f.to_string();
        if line.len() > 400 {
            let cut = (0..=400)
                .rev()
                .find(|&i| line.is_char_boundary(i))
                .unwrap_or(0);
            line.truncate(cut);
            line.push_str(" …");
        }
        println!("  {line}");
    }
}

fn cmd_check(a: &mut Args) -> Result<bool, String> {
    let dir = PathBuf::from(a.value("--corpus").ok_or("--corpus is required")?);
    let which = a.value("--plugin").ok_or("--plugin is required")?;
    let show: usize = a
        .value("--show")
        .map_or(Ok(20), |s| s.parse().map_err(|_| "bad --show"))?;
    let max_findings: usize = a
        .value("--max-findings")
        .map_or(Ok(0), |s| s.parse().map_err(|_| "bad --max-findings"))?;
    let require_python = a.flag("--require-python");
    let plugin: Box<dyn KernelUnderTest> = match which.as_str() {
        "reference" => Box::new(RustReference),
        "toy" => Box::new(Toy::default()),
        s if s.starts_with("toy:") => Box::new(Toy {
            bug: Some(Bug::parse(&s[4..]).ok_or_else(|| format!("unknown bug {}", &s[4..]))?),
        }),
        "external" => {
            let program = PathBuf::from(a.value("--program").ok_or("--program is required")?);
            let threads: Vec<usize> = a.list("--threads", &[1, 4])?;
            let chunks: Vec<u64> = a.list("--chunks", &[64, 1 << 16])?;
            let regimes = a
                .value("--regimes")
                .unwrap_or_else(|| "in_memory".into())
                .split(',')
                .map(regime)
                .collect::<Result<Vec<_>, _>>()?;
            Box::new(External {
                name: program
                    .file_name()
                    .map_or_else(|| "external".into(), |f| f.to_string_lossy().into_owned()),
                args: a.values("--arg"),
                kernels: a
                    .value("--kernels")
                    .ok_or("--kernels is required")?
                    .split(',')
                    .map(str::to_owned)
                    .collect(),
                directed: a.flag("--directed"),
                keeps_self_loops: a.flag("--keeps-self-loops"),
                weighted: a.flag("--weighted"),
                configs: Config::grid(&threads, &chunks, &[Storage::Raw], &regimes),
                program,
            })
        }
        other => return Err(format!("unknown plug-in {other}")),
    };
    a.finish()?;
    let graphs = load_corpus(&dir)?;
    if require_python {
        let missing = graphs.iter().filter(|g| g.references.is_none()).count();
        if missing > 0 {
            return Err(format!(
                "{missing} graphs have no reference.json (run `oracle-ref run` first)"
            ));
        }
    }
    let options = Options {
        skip_rust_reference: which == "reference",
        max_findings,
    };
    let t = Instant::now();
    let report = plugin::run(plugin.as_ref(), &graphs, options);
    summarize(&plugin.name(), &report, show, t.elapsed().as_secs_f64());
    Ok(report.findings.is_empty() && (!require_python || report.python_missing == 0))
}

fn cmd_seeded_bugs(a: &mut Args) -> Result<bool, String> {
    let dir = a.value("--corpus");
    a.finish()?;
    let graphs = match dir {
        Some(d) => load_corpus(Path::new(&d))?,
        None => corpus::entries(Tier::Ci)
            .into_iter()
            .map(Prepared::new)
            .collect(),
    };
    let t = Instant::now();
    let clean = plugin::run(&Toy::default(), &graphs, Options::default());
    println!(
        "toy without a bug: {} pairs checked, {} findings ({:.1} s)",
        clean.checked,
        clean.findings.len(),
        t.elapsed().as_secs_f64()
    );
    let mut all_caught = clean.findings.is_empty();
    println!("\n| seeded bug | checks that fired (number of findings) | families |");
    println!("|---|---|---|");
    for bug in Bug::ALL {
        let report = plugin::run(&Toy { bug: Some(bug) }, &graphs, Options::default());
        let fired: BTreeMap<String, usize> = report.by_check();
        let families = report.families();
        let cells: Vec<String> = fired.iter().map(|(c, n)| format!("`{c}` ({n})")).collect();
        let fams: Vec<&str> = families.keys().copied().collect();
        println!(
            "| {} | {} | {} |",
            bug.name(),
            cells.join(", "),
            fams.join(", ")
        );
        all_caught &= !fired.is_empty();
    }
    Ok(all_caught)
}

fn cmd_golden(a: &mut Args) -> Result<bool, String> {
    let dir = PathBuf::from(a.value("--dir").ok_or("--dir is required")?);
    a.finish()?;
    let results = everygraph_oracle::golden::check_dir(&dir).map_err(|e| e.to_string())?;
    let mut ok = true;
    for r in &results {
        match &r.outcome {
            Ok(checked) => println!("{}: {checked} values agree with the golden file", r.name),
            Err(e) if e.starts_with("skipped") => println!("{}: {e}", r.name),
            Err(e) => {
                ok = false;
                println!("{}: MISMATCH {e}", r.name);
            }
        }
    }
    Ok(ok)
}
