// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The documents of the command checked against their schemas, closed: a missing
//! required key, an unknown key or a value outside an enumeration is an error naming
//! its path. The plan and the error (`docs/ux-contract.md` § 3.6, § 6.1), the file
//! document (§ 4.7), and the result and info documents (`docs/api-sketch.md` § 5.3).

use serde_json::{Map, Value};

type Check = Result<(), String>;

fn object<'v>(
    v: &'v Value,
    at: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<&'v Map<String, Value>, String> {
    let map = v.as_object().ok_or(format!("{at}: not an object"))?;
    for key in required {
        if !map.contains_key(*key) {
            return Err(format!("{at}: {key} missing"));
        }
    }
    for key in map.keys() {
        if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
            return Err(format!("{at}: unknown key {key}"));
        }
    }
    Ok(map)
}

fn one_of(v: &Value, at: &str, values: &[&str]) -> Check {
    match v.as_str() {
        Some(s) if values.contains(&s) => Ok(()),
        _ => Err(format!("{at}: {v} is not one of {values:?}")),
    }
}

fn count(v: &Value, at: &str) -> Check {
    v.as_u64()
        .map(|_| ())
        .ok_or(format!("{at}: {v} is not a count"))
}

fn string(v: &Value, at: &str) -> Check {
    v.as_str()
        .map(|_| ())
        .ok_or(format!("{at}: {v} is not a string"))
}

fn boolean(v: &Value, at: &str) -> Check {
    v.as_bool()
        .map(|_| ())
        .ok_or(format!("{at}: {v} is not a boolean"))
}

fn estimate(v: &Value, at: &str) -> Check {
    let m = object(v, at, &["value", "low", "high", "basis"], &[])?;
    for key in ["value", "low", "high"] {
        if !m[key].as_f64().is_some_and(|x| x >= 0.0) {
            return Err(format!("{at}.{key}: not a number at least 0"));
        }
    }
    one_of(
        &m["basis"],
        &format!("{at}.basis"),
        &["exact", "sampled", "model"],
    )
}

const ORIGINS: &[&str] = &["argument", "graph", "environment", "default"];
const REGIMES: &[&str] = &["in_memory", "mapped", "streamed", "on_disk"];

/// Checks a plan document.
pub fn plan(v: &Value) -> Check {
    let m = object(
        v,
        "plan",
        &[
            "schema",
            "operation",
            "parameters",
            "machine",
            "budget",
            "regime",
            "phases",
            "estimate",
            "scratch",
            "output",
            "fits",
            "refusal",
        ],
        &["call", "input", "graph", "detected", "steps"],
    )?;
    if m["schema"] != 1 {
        return Err("plan.schema: not 1".into());
    }
    string(&m["operation"], "plan.operation")?;
    if !m["parameters"].is_object() {
        return Err("plan.parameters: not an object".into());
    }
    if let Some(input) = m.get("input").filter(|i| !i.is_null()) {
        let i = object(
            input,
            "plan.input",
            &["path", "bytes", "format", "rows"],
            &["compression"],
        )?;
        string(&i["path"], "plan.input.path")?;
        count(&i["bytes"], "plan.input.bytes")?;
        string(&i["format"], "plan.input.format")?;
        estimate(&i["rows"], "plan.input.rows")?;
        if let Some(c) = i.get("compression") {
            one_of(c, "plan.input.compression", &["none", "gzip", "zstd"])?;
        }
    }
    if m.get("graph").is_some_and(|g| !g.is_null()) {
        return Err("plan.graph: an ingest has no graph yet".into());
    }
    if let Some(detected) = m.get("detected").filter(|d| !d.is_null()) {
        let d = object(detected, "plan.detected", &["ids", "edges", "columns"], &[])?;
        let ids = object(
            &d["ids"],
            "plan.detected.ids",
            &["kind", "distinct"],
            &["width_bytes", "noun"],
        )?;
        one_of(
            &ids["kind"],
            "plan.detected.ids.kind",
            &["integer", "hex", "text"],
        )?;
        estimate(&ids["distinct"], "plan.detected.ids.distinct")?;
        let e = object(
            &d["edges"],
            "plan.detected.edges",
            &["directed", "directed_from", "duplicates", "self_loops"],
            &[],
        )?;
        boolean(&e["directed"], "plan.detected.edges.directed")?;
        one_of(
            &e["directed_from"],
            "plan.detected.edges.directed_from",
            &["caller", "column names", "default"],
        )?;
        one_of(
            &e["duplicates"],
            "plan.detected.edges.duplicates",
            &["kept as events", "collapsed", "summed"],
        )?;
        one_of(
            &e["self_loops"],
            "plan.detected.edges.self_loops",
            &["dropped", "kept"],
        )?;
        for (k, c) in d["columns"]
            .as_array()
            .ok_or("plan.detected.columns: not an array")?
            .iter()
            .enumerate()
        {
            let at = format!("plan.detected.columns[{k}]");
            let c = object(c, &at, &["name", "role"], &["type"])?;
            string(&c["name"], &at)?;
            one_of(
                &c["role"],
                &at,
                &["source", "target", "weight", "time", "ignored"],
            )?;
        }
    }
    let machine = object(
        &m["machine"],
        "plan.machine",
        &[
            "cores",
            "hardware_threads",
            "memory_bytes",
            "memory_free_bytes",
            "memory_limit_bytes",
            "storage",
        ],
        &[],
    )?;
    for key in [
        "cores",
        "hardware_threads",
        "memory_bytes",
        "memory_free_bytes",
    ] {
        count(&machine[key], &format!("plan.machine.{key}"))?;
    }
    if !machine["memory_limit_bytes"].is_null() {
        count(
            &machine["memory_limit_bytes"],
            "plan.machine.memory_limit_bytes",
        )?;
    }
    let storage = object(
        &machine["storage"],
        "plan.machine.storage",
        &["kind", "free_bytes"],
        &[],
    )?;
    string(&storage["kind"], "plan.machine.storage.kind")?;
    count(&storage["free_bytes"], "plan.machine.storage.free_bytes")?;
    let budget = object(
        &m["budget"],
        "plan.budget",
        &[
            "memory_bytes",
            "memory_from",
            "above_free",
            "threads",
            "threads_from",
        ],
        &[],
    )?;
    count(&budget["memory_bytes"], "plan.budget.memory_bytes")?;
    one_of(&budget["memory_from"], "plan.budget.memory_from", ORIGINS)?;
    boolean(&budget["above_free"], "plan.budget.above_free")?;
    count(&budget["threads"], "plan.budget.threads")?;
    one_of(&budget["threads_from"], "plan.budget.threads_from", ORIGINS)?;
    let regime = object(
        &m["regime"],
        "plan.regime",
        &["name", "reason", "working_bytes", "ratio"],
        &[],
    )?;
    one_of(&regime["name"], "plan.regime.name", REGIMES)?;
    string(&regime["reason"], "plan.regime.reason")?;
    count(&regime["working_bytes"], "plan.regime.working_bytes")?;
    if !regime["ratio"].as_f64().is_some_and(|r| r >= 0.0) {
        return Err("plan.regime.ratio: not a number at least 0".into());
    }
    let phases = m["phases"]
        .as_array()
        .filter(|p| !p.is_empty())
        .ok_or("plan.phases: not a non-empty array")?;
    for (k, p) in phases.iter().enumerate() {
        let at = format!("plan.phases[{k}]");
        let p = object(p, &at, &["name", "kept", "seconds"], &["regime"])?;
        let name = p["name"].as_str().unwrap_or_default();
        if name.is_empty() || name.len() > 11 || !name.bytes().all(|b| b.is_ascii_lowercase()) {
            return Err(format!("{at}.name: {name:?} is not ^[a-z]{{1,11}}$"));
        }
        boolean(&p["kept"], &at)?;
        estimate(&p["seconds"], &format!("{at}.seconds"))?;
        if let Some(r) = p.get("regime") {
            one_of(r, &format!("{at}.regime"), REGIMES)?;
        }
    }
    let e = object(
        &m["estimate"],
        "plan.estimate",
        &[
            "seconds",
            "peak_memory_bytes",
            "scratch_bytes",
            "output_bytes",
        ],
        &[],
    )?;
    for (key, value) in e {
        estimate(value, &format!("plan.estimate.{key}"))?;
    }
    let scratch = object(
        &m["scratch"],
        "plan.scratch",
        &["path", "from", "memory_backed", "free_bytes"],
        &[],
    )?;
    string(&scratch["path"], "plan.scratch.path")?;
    one_of(&scratch["from"], "plan.scratch.from", ORIGINS)?;
    boolean(&scratch["memory_backed"], "plan.scratch.memory_backed")?;
    count(&scratch["free_bytes"], "plan.scratch.free_bytes")?;
    let output = object(
        &m["output"],
        "plan.output",
        &["path", "kind"],
        &["free_bytes"],
    )?;
    if !output["path"].is_null() {
        string(&output["path"], "plan.output.path")?;
    }
    one_of(
        &output["kind"],
        "plan.output.kind",
        &["graph", "map", "result", "file", "none"],
    )?;
    if let Some(free) = output.get("free_bytes") {
        count(free, "plan.output.free_bytes")?;
    }
    boolean(&m["fits"], "plan.fits")?;
    if !m["refusal"].is_null() {
        error(&m["refusal"], "plan.refusal")?;
    }
    Ok(())
}

/// Checks an error document (§ 6.1).
pub fn error(v: &Value, at: &str) -> Check {
    let m = object(
        v,
        at,
        &["schema", "error", "message", "ways_forward", "changed"],
        &[
            "why",
            "step",
            "needs_bytes",
            "budget_bytes",
            "smallest_budget_bytes",
            "available_bytes",
            "machine_memory_bytes",
            "arithmetic",
            "path",
            "holds",
            "section",
            "column",
            "node",
            "operation",
            "reason",
            "expression",
            "position",
            "problems",
            "invalid_rows",
            "rows_checked",
            "kept",
        ],
    )?;
    one_of(
        &m["error"],
        &format!("{at}.error"),
        &[
            "InsufficientBudget",
            "InsufficientDisk",
            "InvalidInput",
            "InvalidExpression",
            "UnknownNode",
            "Unsupported",
            "OutputExists",
            "CorruptFile",
            "IoError",
            "UsageError",
            "EverygraphError",
        ],
    )?;
    string(&m["message"], &format!("{at}.message"))?;
    boolean(&m["changed"], &format!("{at}.changed"))?;
    for (k, w) in m["ways_forward"]
        .as_array()
        .ok_or(format!("{at}.ways_forward: not an array"))?
        .iter()
        .enumerate()
    {
        let wat = format!("{at}.ways_forward[{k}]");
        let w = object(w, &wat, &["command", "python", "note"], &["needs_bytes"])?;
        for key in ["command", "python", "note"] {
            string(&w[key], &format!("{wat}.{key}"))?;
        }
    }
    let needs: &[&str] = match m["error"].as_str() {
        Some("InsufficientBudget") => &[
            "step",
            "needs_bytes",
            "budget_bytes",
            "smallest_budget_bytes",
        ],
        Some("InsufficientDisk") => &["step", "needs_bytes", "available_bytes", "path"],
        Some("UnknownNode") => &["node"],
        Some("OutputExists") => &["path", "holds"],
        Some("IoError") => &["path"],
        _ => &[],
    };
    for key in needs {
        if !m.contains_key(*key) {
            return Err(format!("{at}: {key} missing for {}", m["error"]));
        }
    }
    if let Some(holds) = m.get("holds") {
        one_of(holds, &format!("{at}.holds"), &["other graph", "not ours"])?;
    }
    Ok(())
}

fn counts(v: &Value, at: &str) -> Check {
    let m = v.as_object().ok_or(format!("{at}: not an object"))?;
    m.iter()
        .try_for_each(|(k, v)| count(v, &format!("{at}.{k}")))
}

fn null_or<F: Fn(&Value, &str) -> Check>(v: &Value, at: &str, f: F) -> Check {
    if v.is_null() { Ok(()) } else { f(v, at) }
}

/// Checks a file document (`urn:everygraph:schema:file:1`).
pub fn file(v: &Value) -> Check {
    let m = object(
        v,
        "file",
        &[
            "schema",
            "job",
            "operation",
            "made",
            "counts",
            "dropped",
            "kept",
            "elapsed_seconds",
        ],
        &[],
    )?;
    if m["schema"] != 1 {
        return Err("file.schema: not 1".into());
    }
    let job = m["job"].as_str().unwrap_or_default();
    if job.len() != 16
        || !job
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!("file.job: {job:?} is not 16 hexadecimal digits"));
    }
    string(&m["operation"], "file.operation")?;
    let made = object(
        &m["made"],
        "file.made",
        &["path", "kind", "bytes"],
        &["reveal"],
    )?;
    string(&made["path"], "file.made.path")?;
    one_of(&made["kind"], "file.made.kind", &["graph", "map", "file"])?;
    count(&made["bytes"], "file.made.bytes")?;
    counts(&m["counts"], "file.counts")?;
    counts(&m["dropped"], "file.dropped")?;
    let kept = m["kept"].as_array().ok_or("file.kept: not an array")?;
    kept.iter().try_for_each(|k| string(k, "file.kept[]"))?;
    if !m["elapsed_seconds"].as_f64().is_some_and(|s| s >= 0.0) {
        return Err("file.elapsed_seconds: not a number at least 0".into());
    }
    Ok(())
}

/// Checks a result document (`urn:everygraph:schema:result:1`).
pub fn result(v: &Value) -> Check {
    let m = object(
        v,
        "result",
        &[
            "schema",
            "result",
            "graph",
            "view",
            "summary",
            "columns",
            "rows",
            "rows_returned",
            "rows_total",
            "stored_in",
            "cached",
            "truncated",
            "meta",
        ],
        &[],
    )?;
    if m["schema"] != 1 {
        return Err("result.schema: not 1".into());
    }
    string(&m["result"], "result.result")?;
    string(&m["graph"], "result.graph")?;
    null_or(&m["view"], "result.view", |v, at| {
        object(v, at, &[], &["window", "edges", "nodes"]).map(|_| ())
    })?;
    if !(m["summary"].is_null() || m["summary"].is_object()) {
        return Err("result.summary: not an object or null".into());
    }
    let columns = m["columns"]
        .as_array()
        .ok_or("result.columns: not an array")?;
    columns
        .iter()
        .try_for_each(|c| string(c, "result.columns[]"))?;
    let rows = m["rows"].as_array().ok_or("result.rows: not an array")?;
    for (k, row) in rows.iter().enumerate() {
        let row = row
            .as_array()
            .ok_or(format!("result.rows[{k}]: not an array"))?;
        if row.len() != columns.len() {
            return Err(format!("result.rows[{k}]: not one value per column"));
        }
    }
    count(&m["rows_returned"], "result.rows_returned")?;
    count(&m["rows_total"], "result.rows_total")?;
    if m["rows_returned"].as_u64() != Some(rows.len() as u64) {
        return Err("result.rows_returned: not the number of rows".into());
    }
    null_or(&m["stored_in"], "result.stored_in", string)?;
    boolean(&m["cached"], "result.cached")?;
    null_or(&m["truncated"], "result.truncated", |v, at| {
        let t = object(
            v,
            at,
            &["by", "limit", "rows", "where"],
            &["timeout_seconds", "total"],
        )?;
        one_of(&t["by"], &format!("{at}.by"), &["limit", "timeout"])
    })?;
    if !m["meta"].is_object() {
        return Err("result.meta: not an object".into());
    }
    Ok(())
}

/// Checks an info document (`urn:everygraph:schema:info:1`).
pub fn info(v: &Value) -> Check {
    let m = object(
        v,
        "info",
        &[
            "schema",
            "info",
            "bytes",
            "generation",
            "graph",
            "build",
            "reclaimable_bytes",
            "sections",
            "results",
        ],
        &[],
    )?;
    if m["schema"] != 1 {
        return Err("info.schema: not 1".into());
    }
    string(&m["info"], "info.info")?;
    for key in ["bytes", "generation", "reclaimable_bytes"] {
        count(&m[key], &format!("info.{key}"))?;
    }
    let g = object(
        &m["graph"],
        "info.graph",
        &[
            "nodes",
            "edges",
            "directed",
            "self_loops",
            "duplicates",
            "ids",
        ],
        &[],
    )?;
    count(&g["nodes"], "info.graph.nodes")?;
    count(&g["edges"], "info.graph.edges")?;
    boolean(&g["directed"], "info.graph.directed")?;
    null_or(&g["self_loops"], "info.graph.self_loops", |v, at| {
        one_of(v, at, &["drop", "keep"])
    })?;
    null_or(&g["duplicates"], "info.graph.duplicates", |v, at| {
        one_of(v, at, &["events", "collapse", "sum"])
    })?;
    string(&g["ids"], "info.graph.ids")?;
    let b = object(
        &m["build"],
        "info.build",
        &[
            "engine", "finished", "seconds", "inputs", "counts", "dropped",
        ],
        &[],
    )?;
    string(&b["engine"], "info.build.engine")?;
    null_or(&b["finished"], "info.build.finished", string)?;
    null_or(&b["seconds"], "info.build.seconds", |v, at| {
        v.as_f64()
            .filter(|s| *s >= 0.0)
            .map(|_| ())
            .ok_or(format!("{at}: not a number"))
    })?;
    for (k, i) in b["inputs"]
        .as_array()
        .ok_or("info.build.inputs: not an array")?
        .iter()
        .enumerate()
    {
        let at = format!("info.build.inputs[{k}]");
        let i = object(i, &at, &["name", "bytes"], &[])?;
        string(&i["name"], &at)?;
        count(&i["bytes"], &at)?;
    }
    counts(&b["counts"], "info.build.counts")?;
    counts(&b["dropped"], "info.build.dropped")?;
    for (k, s) in m["sections"]
        .as_array()
        .ok_or("info.sections: not an array")?
        .iter()
        .enumerate()
    {
        let at = format!("info.sections[{k}]");
        let s = object(s, &at, &["name", "kind", "bytes"], &[])?;
        string(&s["name"], &at)?;
        string(&s["kind"], &at)?;
        count(&s["bytes"], &at)?;
    }
    for (k, r) in m["results"]
        .as_array()
        .ok_or("info.results: not an array")?
        .iter()
        .enumerate()
    {
        let at = format!("info.results[{k}]");
        let r = object(
            r,
            &at,
            &[
                "operation",
                "section",
                "bytes",
                "engine",
                "finished",
                "seconds",
                "parameters",
                "stale",
            ],
            &[],
        )?;
        string(&r["operation"], &at)?;
        string(&r["section"], &at)?;
        count(&r["bytes"], &at)?;
        string(&r["engine"], &at)?;
        null_or(&r["finished"], &at, string)?;
        let p = r["parameters"]
            .as_object()
            .ok_or(format!("{at}.parameters: not an object"))?;
        p.values()
            .try_for_each(|v| string(v, &format!("{at}.parameters")))?;
        null_or(&r["stale"], &format!("{at}.stale"), |v, at| {
            one_of(v, at, &["graph", "version"])
        })?;
    }
    Ok(())
}
