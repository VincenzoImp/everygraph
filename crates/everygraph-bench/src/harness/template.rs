// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `{name}` placeholders in spec strings. `{{` and `}}` stand for literal braces.

use std::collections::BTreeMap;

/// Replaces each `{name}` in `template` by what `lookup(name)` returns.
///
/// # Errors
///
/// What `lookup` returns for a name it does not know, an unclosed `{`, or a lone `}`.
pub fn expand(
    template: &str,
    lookup: &mut dyn FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(t) = tail.strip_prefix("{{") {
            out.push('{');
            rest = t;
        } else if let Some(t) = tail.strip_prefix("}}") {
            out.push('}');
            rest = t;
        } else if tail.starts_with('}') {
            return Err(format!(
                "a lone '}}' in {template:?}; write '}}}}' for a brace"
            ));
        } else {
            let end = tail
                .find('}')
                .ok_or_else(|| format!("an unclosed '{{' in {template:?}"))?;
            out.push_str(&lookup(&tail[1..end])?);
            rest = &tail[end + 1..];
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// Placeholder values: fixed ones (built-ins, matrix values) and the spec's variables,
/// whose own text may refer to other names.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    fixed: BTreeMap<String, String>,
    vars: BTreeMap<String, String>,
}

impl Scope {
    /// A scope with these variables (each a template itself).
    #[must_use]
    pub fn new(vars: BTreeMap<String, String>) -> Self {
        Self {
            fixed: BTreeMap::new(),
            vars,
        }
    }

    /// Sets a fixed value; it takes precedence over a variable of the same name.
    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        self.fixed.insert(name.to_string(), value.into());
    }

    /// Expands `template` in this scope.
    ///
    /// # Errors
    ///
    /// An unknown name, or a variable that refers to itself, directly or not.
    pub fn expand(&self, template: &str) -> Result<String, String> {
        self.expand_inner(template, &mut Vec::new())
    }

    fn expand_inner(&self, template: &str, active: &mut Vec<String>) -> Result<String, String> {
        expand(template, &mut |name: &str| {
            if let Some(v) = self.fixed.get(name) {
                return Ok(v.clone());
            }
            let Some(text) = self.vars.get(name) else {
                return Err(format!(
                    "{template:?} uses {{{name}}}, which is neither a built-in nor in [vars]"
                ));
            };
            if active.iter().any(|a| a == name) {
                return Err(format!("variable {{{name}}} refers to itself"));
            }
            active.push(name.to_string());
            let value = self.expand_inner(text, active);
            active.pop();
            value
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn placeholders_and_escapes() {
        let mut scope = Scope::default();
        scope.set("t", "32");
        assert_eq!(scope.expand("--threads {t}"), Ok("--threads 32".into()));
        assert_eq!(scope.expand("{{literal}} {t}"), Ok("{literal} 32".into()));
        assert!(scope.expand("{nope}").unwrap_err().contains("{nope}"));
        assert!(scope.expand("{t").unwrap_err().contains("unclosed"));
        assert!(scope.expand("t}").unwrap_err().contains("lone"));
    }

    #[test]
    fn variables_refer_to_fixed_values_and_to_each_other() {
        let mut scope = Scope::new(map(&[
            ("glab", "{repo}/lab/glab"),
            ("run", "{glab} run --threads {threads}"),
        ]));
        scope.set("repo", "/r");
        scope.set("threads", "8");
        assert_eq!(
            scope.expand("{run}"),
            Ok("/r/lab/glab run --threads 8".into())
        );
    }

    #[test]
    fn cycles_are_reported() {
        let scope = Scope::new(map(&[("a", "{b}"), ("b", "{a}")]));
        let err = scope.expand("{a}").unwrap_err();
        assert!(err.contains("refers to itself"), "{err}");
    }
}
