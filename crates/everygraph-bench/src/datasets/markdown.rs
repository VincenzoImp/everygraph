// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The tables of `datasets/README.md`, generated from the registry by
//! `everygraph-bench list --markdown`. A test checks that the README holds exactly this
//! output, so the two cannot drift apart.

use std::fmt::Write as _;

use super::registry::{Dataset, Registry};
use super::store::human_bytes;

/// The marker lines between which `datasets/README.md` holds the generated text.
pub const BEGIN: &str = "<!-- BEGIN GENERATED: everygraph-bench list --markdown -->";
/// See [`BEGIN`].
pub const END: &str = "<!-- END GENERATED -->";

/// The generated part of the README: the summary table and one section per dataset.
#[must_use]
pub fn render(registry: &Registry) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{BEGIN}");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "Values marked † are not verified against the source; see each dataset's notes."
    );
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "| Name | Tier | Family | Dir. | Wt. | Time | Nodes | Edges | Download | Checksum | Licence / terms |"
    );
    let _ = writeln!(s, "|---|---|---|---|---|---|--:|--:|--:|---|---|");
    for d in &registry.datasets {
        let name = match &d.page {
            Some(page) => format!("[`{}`]({page})", d.name),
            None => format!("`{}`", d.name),
        };
        let licence = match &d.licence_url {
            Some(url) => format!("[{}]({url})", short(&d.licence)),
            None => short(&d.licence),
        };
        let download = if d.is_generated() {
            "generated".to_string()
        } else {
            d.total_size().map_or_else(|| "?".to_string(), human_bytes)
        };
        let checksum = if d.files.iter().all(|f| f.sha256.is_some()) {
            "SHA-256"
        } else if d.files.iter().all(|f| f.md5_published.is_some()) {
            "MD5 (source) only †"
        } else {
            "none †"
        };
        let _ = writeln!(
            s,
            "| {name} | {} | {} | {} | {} | {} | {} | {} | {download} | {checksum} | {}{} |",
            d.tier,
            d.family,
            flag(d.directed, d.is_unverified("directed")),
            flag(d.weighted, d.is_unverified("weighted")),
            flag(d.temporal, d.is_unverified("temporal")),
            count(d.nodes, d.is_unverified("nodes")),
            count(d.edges, d.is_unverified("edges")),
            licence,
            mark(d.is_unverified("licence")),
        );
    }
    let _ = writeln!(s);
    for d in &registry.datasets {
        details(&mut s, d);
    }
    let _ = writeln!(s, "{END}");
    s
}

/// Replaces the text between [`BEGIN`] and [`END`] (markers included) in `document` with
/// `generated`; `None` if the markers are missing.
#[must_use]
pub fn splice(document: &str, generated: &str) -> Option<String> {
    let start = document.find(BEGIN)?;
    let end = document.get(start..)?.find(END)? + start + END.len();
    let mut out = String::with_capacity(document.len() + generated.len());
    out.push_str(document.get(..start)?);
    out.push_str(generated.trim_end());
    out.push_str(document.get(end..)?);
    Some(out)
}

fn details(s: &mut String, d: &Dataset) {
    let _ = writeln!(s, "### `{}`", d.name);
    let _ = writeln!(s);
    let _ = writeln!(s, "{}", d.title);
    let _ = writeln!(s);
    let _ = writeln!(s, "- **Format:** {}", d.format);
    let _ = writeln!(s, "- **To an edge list:** {}", d.to_edge_list);
    if let Some(note) = &d.counts_note {
        let _ = writeln!(s, "- **Counts:** {note}");
    }
    let _ = writeln!(s, "- **Licence / terms:** {}", d.licence);
    if let Some(c) = &d.citation {
        let _ = writeln!(s, "- **Cite:** {c}");
    }
    if let Some(n) = &d.notes {
        let _ = writeln!(s, "- **Notes:** {n}");
    }
    if !d.unverified.is_empty() {
        let _ = writeln!(s, "- **Not verified:** {}", d.unverified.join(", "));
    }
    for f in &d.files {
        let what = match &f.url {
            Some(url) => format!("[`{}`]({url})", f.name),
            None => format!("`{}` (generated)", f.name),
        };
        let size = f
            .size
            .map_or_else(|| "size unknown".to_string(), |n| format!("{n} bytes"));
        let sha = f.sha256.as_deref().map_or_else(
            || "SHA-256 not pinned".to_string(),
            |h| format!("SHA-256 `{h}`"),
        );
        let _ = writeln!(s, "- **File:** {what}, {size}, {sha}");
    }
    let _ = writeln!(s);
}

fn short(licence: &str) -> String {
    // The first sentence; the full text is in the dataset's section.
    let first = licence.split(". ").next().unwrap_or(licence);
    first.trim_end_matches('.').replace('|', "\\|")
}

fn flag(value: bool, unverified: bool) -> String {
    format!("{}{}", if value { "yes" } else { "no" }, mark(unverified))
}

fn count(value: Option<u64>, unverified: bool) -> String {
    let text = value.map_or_else(|| "?".to_string(), thousands);
    format!("{text}{}", mark(unverified))
}

fn mark(unverified: bool) -> &'static str {
    if unverified { " †" } else { "" }
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_readme_holds_the_generated_tables() {
        let readme = include_str!("../../../../datasets/README.md");
        let registry = Registry::embedded().expect("valid");
        let generated = render(&registry);
        let start = readme.find(BEGIN).expect("README has the BEGIN marker");
        let end = readme.find(END).expect("README has the END marker") + END.len();
        let current = readme.get(start..end).expect("in bounds");
        assert!(
            current == generated.trim_end(),
            "datasets/README.md is out of date: run `everygraph-bench list --markdown` and \
             paste its output between the markers"
        );
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1_806_067_135), "1,806,067,135");
    }
}
