// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Numbers read from a command's output, as a spec's `metric` entries say.

use std::collections::BTreeMap;

use serde_json::Value;

use super::spec::{MetricDef, Reduce, Stream};

/// The last line of `text` that is a JSON object.
#[must_use]
pub fn last_json_object(text: &str) -> Option<Value> {
    text.lines().rev().find_map(|l| {
        let l = l.trim();
        if !l.starts_with('{') {
            return None;
        }
        serde_json::from_str::<Value>(l)
            .ok()
            .filter(Value::is_object)
    })
}

fn reduce(values: &[f64], how: Reduce) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss, reason = "array lengths far below 2^53")]
    let n = values.len() as f64;
    Some(match how {
        Reduce::Min => values.iter().copied().fold(f64::INFINITY, f64::min),
        Reduce::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Reduce::Sum => values.iter().sum(),
        Reduce::Mean => values.iter().sum::<f64>() / n,
        Reduce::Median => median(values),
    })
}

/// The median (mean of the middle two for an even count). `values` must not be empty.
#[must_use]
pub fn median(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    if v.len() % 2 == 1 {
        v[mid]
    } else {
        (v[mid - 1] + v[mid]) / 2.0
    }
}

fn from_json(doc: Option<&Value>, m: &MetricDef, pointer: &str) -> Result<f64, String> {
    let doc = doc.ok_or("no line of the output is a JSON object")?;
    let v = doc
        .pointer(pointer)
        .ok_or_else(|| format!("the JSON output has nothing at {pointer:?}"))?;
    match v {
        Value::Number(n) => n
            .as_f64()
            .ok_or_else(|| format!("{pointer:?} is not a finite number")),
        Value::Array(items) => {
            let nums: Option<Vec<f64>> = items.iter().map(Value::as_f64).collect();
            let nums =
                nums.ok_or_else(|| format!("{pointer:?} is an array with a non-number in it"))?;
            let how = m.reduce.ok_or_else(|| {
                format!("{pointer:?} is an array: say how to reduce it (reduce = \"median\", …)")
            })?;
            reduce(&nums, how).ok_or_else(|| format!("{pointer:?} is an empty array"))
        }
        other => Err(format!("{pointer:?} is {other}, not a number")),
    }
}

fn from_regex(text: &str, pattern: &str) -> Result<f64, String> {
    let re = regex_lite::Regex::new(pattern).map_err(|e| e.to_string())?;
    let caps = re
        .captures(text)
        .ok_or_else(|| format!("nothing in the output matches {pattern:?}"))?;
    let g = caps.get(1).map_or("", |g| g.as_str());
    g.trim()
        .parse()
        .map_err(|_| format!("{pattern:?} captured {g:?}, which is not a number"))
}

/// The metrics of `defs`, read from `stdout` and `stderr`. Missing optional metrics are
/// left out; the second value lists the required ones that could not be read.
#[must_use]
pub fn extract(
    defs: &[MetricDef],
    stdout: &str,
    stderr: &str,
) -> (BTreeMap<String, f64>, Vec<String>) {
    let docs = (last_json_object(stdout), last_json_object(stderr));
    let mut values = BTreeMap::new();
    let mut errors = Vec::new();
    for m in defs {
        let (text, doc) = match m.from {
            Stream::Stdout => (stdout, docs.0.as_ref()),
            Stream::Stderr => (stderr, docs.1.as_ref()),
        };
        let raw = match (&m.json, &m.regex) {
            (Some(p), _) => from_json(doc, m, p),
            (None, Some(r)) => from_regex(text, r),
            (None, None) => Err("neither json nor regex".into()),
        };
        match raw {
            Ok(v) => {
                let v = v * m.scale.unwrap_or(1.0);
                if v.is_finite() {
                    values.insert(m.name.clone(), v);
                } else {
                    errors.push(format!("metric {}: not a finite number", m.name));
                }
            }
            Err(_) if m.optional => {}
            Err(e) => errors.push(format!("metric {}: {e}", m.name)),
        }
    }
    (values, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(name: &str, json: Option<&str>, regex: Option<&str>) -> MetricDef {
        MetricDef {
            name: name.into(),
            json: json.map(Into::into),
            regex: regex.map(Into::into),
            from: Stream::Stdout,
            reduce: None,
            scale: None,
            unit: None,
            optional: false,
        }
    }

    /// One line of `glab run` output, as the lab prototype prints it.
    const GLAB: &str = r#"progress text
{"algo":"pr32","cmd":"run","seconds_all":[0.41,0.38,0.39],"seconds_best":0.38,"peak_rss_gb":0.94,"result":{"iters":20}}
"#;

    #[test]
    fn json_pointers_scale_and_reduce() {
        let mut gib = metric("peak", Some("/peak_rss_gb"), None);
        gib.scale = Some(1024.0);
        let mut med = metric("median", Some("/seconds_all"), None);
        med.reduce = Some(Reduce::Median);
        let defs = [
            metric("seconds", Some("/seconds_best"), None),
            gib,
            med,
            metric("iters", Some("/result/iters"), None),
        ];
        let (v, errors) = extract(&defs, GLAB, "");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(v["seconds"], 0.38);
        assert!((v["peak"] - 0.94 * 1024.0).abs() < 1e-9);
        assert_eq!(v["median"], 0.39);
        assert_eq!(v["iters"], 20.0);
    }

    #[test]
    fn regexes_and_failures() {
        let mut from_err = metric("ms", None, Some(r"took ([0-9.]+) ms"));
        from_err.from = Stream::Stderr;
        let (v, errors) = extract(&[from_err], "", "it took 12.5 ms\n");
        assert!(errors.is_empty());
        assert_eq!(v["ms"], 12.5);

        let mut optional = metric("maybe", Some("/nope"), None);
        optional.optional = true;
        let defs = [
            metric("missing", Some("/nope"), None),
            metric("array", Some("/seconds_all"), None),
            metric("object", Some("/result"), None),
            optional,
        ];
        let (v, errors) = extract(&defs, GLAB, "");
        assert!(v.is_empty());
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors[1].contains("say how to reduce"));
        let (_, errors) = extract(&[metric("s", Some("/x"), None)], "not json\n", "");
        assert!(errors[0].contains("no line of the output is a JSON object"));
    }

    #[test]
    fn medians() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
    }
}
