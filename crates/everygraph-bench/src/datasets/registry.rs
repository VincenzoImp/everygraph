// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The dataset registry: `datasets/datasets.toml` at the root of the repository, parsed
//! and checked. The file is compiled into the binary, so the tool works from any
//! directory; `--registry` reads another file instead.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use serde::Deserialize;

use super::error::{Context, Error, Result};
use super::synth::Generator;

/// The registry shipped with this build: the text of `datasets/datasets.toml`.
pub const EMBEDDED: &str = include_str!("../../../../datasets/datasets.toml");

/// The version of the file format this code reads.
pub const SCHEMA: u32 = 1;

/// Fields that an entry may declare as not checked against its source.
pub const UNVERIFIABLE_FIELDS: &[&str] = &[
    "nodes", "edges", "directed", "weighted", "temporal", "licence", "sha256", "size", "format",
];

/// The kind of graph a dataset stands for. Every family named in issue #25 (graph
/// families) has at least one dataset; the registry tests check it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
    /// An undirected friendship or membership network.
    Social,
    /// A hyperlink graph.
    Web,
    /// A road network: high diameter, degree about 2 to 4.
    Road,
    /// Two kinds of nodes, edges only between kinds (user–item).
    Bipartite,
    /// Edges carry a numeric weight.
    Weighted,
    /// Edges carry a timestamp; usually a multigraph.
    Temporal,
    /// A directed social network (followers, trust, votes).
    DirectedSocial,
    /// Generated locally by a deterministic generator; never downloaded.
    Synthetic,
    /// Published by a benchmark together with its reference outputs (LDBC Graphalytics).
    Conformance,
}

impl Family {
    /// Every family, in display order.
    pub const ALL: [Family; 9] = [
        Family::Social,
        Family::Web,
        Family::Road,
        Family::Bipartite,
        Family::Weighted,
        Family::Temporal,
        Family::DirectedSocial,
        Family::Synthetic,
        Family::Conformance,
    ];

    /// The name used in the registry file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Family::Social => "social",
            Family::Web => "web",
            Family::Road => "road",
            Family::Bipartite => "bipartite",
            Family::Weighted => "weighted",
            Family::Temporal => "temporal",
            Family::DirectedSocial => "directed-social",
            Family::Synthetic => "synthetic",
            Family::Conformance => "conformance",
        }
    }
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One file of a dataset.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    /// The file name under the dataset's directory; also the name looked for by
    /// `--from-dir`.
    pub name: String,
    /// Where to download it. Absent for generated files.
    pub url: Option<String>,
    /// Other places that serve the same bytes, tried in order after `url`.
    #[serde(default)]
    pub mirrors: Vec<String>,
    /// Size in bytes.
    pub size: Option<u64>,
    /// SHA-256 of the file, lowercase hex. Absent only for tier-3 files nobody has
    /// downloaded yet ("unpinned").
    pub sha256: Option<String>,
    /// An MD5 sum published by the source itself, checked when present.
    pub md5_published: Option<String>,
}

impl FileEntry {
    /// The URL followed by the mirrors.
    #[must_use]
    pub fn urls(&self) -> Vec<&str> {
        self.url
            .iter()
            .chain(self.mirrors.iter())
            .map(String::as_str)
            .collect()
    }
}

/// One dataset of the registry.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dataset {
    /// The name used on the command line: lowercase letters, digits, `-` and `.`.
    pub name: String,
    /// A one-line description, with the source's own name for the graph.
    pub title: String,
    /// The family the dataset stands for in the graph-family experiments.
    pub family: Family,
    /// 1: continuous integration (the whole tier stays under about 1 GB);
    /// 2: workstation; 3: server, optional.
    pub tier: u8,
    /// The edges have a direction.
    pub directed: bool,
    /// The edges carry a weight.
    pub weighted: bool,
    /// The edges carry a timestamp.
    pub temporal: bool,
    /// Node count as published by the source.
    pub nodes: Option<u64>,
    /// Edge count as published by the source (see `counts_note` for what it counts).
    pub edges: Option<u64>,
    /// What the published counts mean, and what was checked against the file.
    pub counts_note: Option<String>,
    /// The file format.
    pub format: String,
    /// How to turn the files into a plain edge list.
    pub to_edge_list: String,
    /// The source's page for the dataset.
    pub page: Option<String>,
    /// Licence or terms of use, as stated by the source.
    pub licence: String,
    /// Where the licence or terms are stated.
    pub licence_url: Option<String>,
    /// What the source asks to be cited.
    pub citation: Option<String>,
    /// Anything else a user must know: required features, caveats.
    pub notes: Option<String>,
    /// Fields not checked against the source; see [`UNVERIFIABLE_FIELDS`].
    #[serde(default)]
    pub unverified: Vec<String>,
    /// The generator, for synthetic datasets.
    pub generator: Option<Generator>,
    /// The files that make up the dataset.
    #[serde(rename = "file")]
    pub files: Vec<FileEntry>,
}

impl Dataset {
    /// Whether the dataset is generated locally rather than downloaded.
    #[must_use]
    pub fn is_generated(&self) -> bool {
        self.generator.is_some()
    }

    /// Whether `field` is declared as not checked against the source.
    #[must_use]
    pub fn is_unverified(&self, field: &str) -> bool {
        self.unverified.iter().any(|f| f == field)
    }

    /// Total size of the files, if every size is known.
    #[must_use]
    pub fn total_size(&self) -> Option<u64> {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// The parsed and checked registry.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    /// The format version; must equal [`SCHEMA`].
    pub schema: u32,
    /// The datasets, in file order.
    #[serde(rename = "dataset")]
    pub datasets: Vec<Dataset>,
}

/// Words that select groups of datasets and therefore cannot be dataset names.
const RESERVED_NAMES: &[&str] = &["all", "tier1", "tier2", "tier3"];

impl Registry {
    /// The registry compiled into this build.
    ///
    /// # Errors
    /// If the embedded file does not parse or fails a check; the registry tests make sure
    /// it does not.
    pub fn embedded() -> Result<Self> {
        Self::parse(EMBEDDED)
    }

    /// Reads and checks a registry file.
    ///
    /// # Errors
    /// If the file cannot be read, does not parse, or fails a check.
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).context(|| format!("in {}", path.display()))
    }

    /// Parses and checks registry text.
    ///
    /// # Errors
    /// If the text is not valid TOML for this schema, or an entry fails a check (see
    /// [`Registry::check`]).
    pub fn parse(text: &str) -> Result<Self> {
        let registry: Registry = toml::from_str(text).map_err(|e| Error::new(e.to_string()))?;
        registry.check()?;
        Ok(registry)
    }

    /// The checks that the type system does not make: unique and well-formed names,
    /// tiers in range, checksums well formed and present where required, generated
    /// datasets consistent, and every unknown count or checksum declared as unverified.
    ///
    /// # Errors
    /// The first failed check, naming the dataset.
    pub fn check(&self) -> Result<()> {
        if self.schema != SCHEMA {
            return Err(Error::new(format!(
                "registry schema {} is not supported (this build reads {SCHEMA})",
                self.schema
            )));
        }
        let mut names = BTreeSet::new();
        for ds in &self.datasets {
            check_dataset(ds).context(|| format!("dataset {:?}", ds.name))?;
            if !names.insert(ds.name.as_str()) {
                return Err(Error::new(format!("dataset {:?} is listed twice", ds.name)));
            }
        }
        Ok(())
    }

    /// The dataset with this name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Dataset> {
        self.datasets.iter().find(|d| d.name == name)
    }

    /// Resolves a selector: a dataset name, `tier1`, `tier2`, `tier3` (each tier
    /// includes the lower ones) or `all`.
    ///
    /// # Errors
    /// If the selector names no dataset.
    pub fn select(&self, selector: &str) -> Result<Vec<&Dataset>> {
        let max_tier = match selector {
            "all" | "tier3" => Some(3),
            "tier2" => Some(2),
            "tier1" => Some(1),
            _ => None,
        };
        if let Some(max_tier) = max_tier {
            return Ok(self
                .datasets
                .iter()
                .filter(|d| d.tier <= max_tier)
                .collect());
        }
        self.get(selector).map(|d| vec![d]).ok_or_else(|| {
            Error::new(format!(
                "no dataset named {selector:?} (try `everygraph-bench list`)"
            ))
        })
    }
}

fn check_dataset(ds: &Dataset) -> Result<()> {
    check_name(&ds.name)?;
    if !(1..=3).contains(&ds.tier) {
        return Err(Error::new(format!("tier {} is not 1, 2 or 3", ds.tier)));
    }
    for field in &ds.unverified {
        if !UNVERIFIABLE_FIELDS.contains(&field.as_str()) {
            return Err(Error::new(format!(
                "unverified lists {field:?}, which is not one of {UNVERIFIABLE_FIELDS:?}"
            )));
        }
    }
    if ds.nodes.is_none() && !ds.is_unverified("nodes") {
        return Err(Error::new("nodes is missing and not listed as unverified"));
    }
    if ds.edges.is_none() && !ds.is_unverified("edges") {
        return Err(Error::new("edges is missing and not listed as unverified"));
    }
    if ds.files.is_empty() {
        return Err(Error::new("no [[dataset.file]] entries"));
    }
    let generated = ds.is_generated();
    // A generated graph may stand for another family (the weighted and bipartite graphs
    // of tier 1); the synthetic family holds generated graphs only.
    if ds.family == Family::Synthetic && !generated {
        return Err(Error::new("the synthetic family needs a generator"));
    }
    if generated && (ds.files.len() != 1 || ds.tier != 1) {
        return Err(Error::new(
            "a generated dataset is tier 1 and has exactly one file",
        ));
    }
    let mut file_names = BTreeSet::new();
    for file in &ds.files {
        check_file(ds, file, generated).context(|| format!("file {:?}", file.name))?;
        if !file_names.insert(file.name.as_str()) {
            return Err(Error::new(format!("file {:?} is listed twice", file.name)));
        }
    }
    Ok(())
}

fn check_name(name: &str) -> Result<()> {
    let well_formed = name
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
    if !well_formed {
        return Err(Error::new(
            "a name starts with a lowercase letter or digit and contains only those, '-' and '.'",
        ));
    }
    if RESERVED_NAMES.contains(&name) {
        return Err(Error::new(format!("{name:?} is a reserved selector")));
    }
    Ok(())
}

fn check_file(ds: &Dataset, file: &FileEntry, generated: bool) -> Result<()> {
    let name_ok = !file.name.is_empty()
        && !file.name.starts_with('.')
        && !file.name.contains(['/', '\\'])
        && file.name != "..";
    if !name_ok {
        return Err(Error::new(
            "a file name is a plain name: no path separators, not starting with '.'",
        ));
    }
    if generated {
        if file.url.is_some() || !file.mirrors.is_empty() {
            return Err(Error::new("a generated file has no URL"));
        }
    } else {
        let Some(url) = &file.url else {
            return Err(Error::new("url is missing"));
        };
        for u in std::iter::once(url).chain(&file.mirrors) {
            if !(u.starts_with("https://") || u.starts_with("http://")) {
                return Err(Error::new(format!("{u:?} is not an http(s) URL")));
            }
        }
    }
    match &file.sha256 {
        Some(h) => check_hex(h, 64, "sha256")?,
        None => {
            if ds.tier < 3 || generated {
                return Err(Error::new(
                    "sha256 is required in tiers 1 and 2 and for generated files",
                ));
            }
            if !ds.is_unverified("sha256") {
                return Err(Error::new("sha256 is missing and not listed as unverified"));
            }
        }
    }
    if let Some(h) = &file.md5_published {
        check_hex(h, 32, "md5_published")?;
    }
    if file.size.is_none() && !ds.is_unverified("size") {
        return Err(Error::new("size is missing and not listed as unverified"));
    }
    Ok(())
}

fn check_hex(value: &str, len: usize, what: &str) -> Result<()> {
    if value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(Error::new(format!(
            "{what} must be {len} lowercase hexadecimal digits"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H64: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn one(extra_dataset: &str, extra_file: &str) -> String {
        format!(
            r#"
schema = 1
[[dataset]]
name = "toy"
title = "Toy"
family = "social"
tier = 1
directed = false
weighted = false
temporal = false
nodes = 3
edges = 2
format = "edge list"
to_edge_list = "as is"
licence = "none"
{extra_dataset}
[[dataset.file]]
name = "toy.txt"
url = "https://example.org/toy.txt"
size = 10
sha256 = "{H64}"
{extra_file}
"#
        )
    }

    fn err(text: &str) -> String {
        Registry::parse(text)
            .expect_err("must be rejected")
            .to_string()
    }

    #[test]
    fn the_embedded_registry_parses_and_passes_every_check() {
        let r = Registry::embedded().expect("datasets/datasets.toml is valid");
        assert!(r.datasets.len() >= 15, "{}", r.datasets.len());
    }

    #[test]
    fn every_family_has_a_registered_dataset() {
        // Acceptance of issue #5: each family of issue #25 has at least one dataset.
        let r = Registry::embedded().expect("valid");
        for family in Family::ALL {
            assert!(
                r.datasets.iter().any(|d| d.family == family),
                "no dataset for {family}"
            );
        }
    }

    #[test]
    fn tier1_covers_the_families_ci_needs_and_stays_small() {
        let r = Registry::embedded().expect("valid");
        let tier1 = r.select("tier1").expect("tier1");
        for family in [
            Family::Social,
            Family::Road,
            Family::Bipartite,
            Family::Weighted,
            Family::Temporal,
            Family::Synthetic,
        ] {
            assert!(
                tier1.iter().any(|d| d.family == family),
                "tier 1 lacks {family}"
            );
        }
        let total: u64 = tier1
            .iter()
            .map(|d| d.total_size().expect("tier-1 sizes are known"))
            .sum();
        assert!(total < 1_000_000_000, "tier 1 is {total} bytes");
    }

    #[test]
    fn tier1_has_generated_weighted_and_bipartite_graphs_ci_can_rely_on() {
        // Positive integer weights and a true two-sided graph, with no third-party terms.
        let r = Registry::embedded().expect("valid");
        let tier1 = r.select("tier1").expect("tier1");
        assert!(tier1.iter().any(|d| d.family == Family::Weighted
            && d.weighted
            && matches!(d.generator, Some(Generator::WeightedGnm { max_weight, .. }) if max_weight >= 1)));
        assert!(tier1.iter().any(|d| d.family == Family::Bipartite
            && matches!(d.generator, Some(Generator::Bipartite { .. }))));
        for d in &r.datasets {
            if d.family == Family::Weighted {
                assert!(d.weighted, "{} stands for the weighted family", d.name);
            }
        }
    }

    #[test]
    fn tiers_one_and_two_are_fully_pinned() {
        let r = Registry::embedded().expect("valid");
        for d in r.select("tier2").expect("tier2") {
            for f in &d.files {
                assert!(f.sha256.is_some(), "{}/{} is unpinned", d.name, f.name);
                assert!(f.size.is_some(), "{}/{} has no size", d.name, f.name);
            }
        }
    }

    #[test]
    fn a_minimal_entry_parses() {
        let r = Registry::parse(&one("", "")).expect("valid");
        let d = r.get("toy").expect("present");
        assert_eq!(d.family, Family::Social);
        assert_eq!(d.files[0].urls(), vec!["https://example.org/toy.txt"]);
        assert_eq!(d.total_size(), Some(10));
    }

    #[test]
    fn selectors_resolve_names_and_cumulative_tiers() {
        let text = one("", "").replace("[[dataset]]", "[[dataset]]\n")
            + &one("", "")
                .replace("schema = 1", "")
                .replace("\"toy\"", "\"big\"")
                .replace("tier = 1", "tier = 2");
        let r = Registry::parse(&text).expect("valid");
        assert_eq!(r.select("tier1").expect("ok").len(), 1);
        assert_eq!(r.select("tier2").expect("ok").len(), 2);
        assert_eq!(r.select("all").expect("ok").len(), 2);
        assert_eq!(r.select("big").expect("ok")[0].name, "big");
        assert!(r.select("nope").is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(err(&one("colour = \"red\"", "")).contains("colour"));
        assert!(err(&one("", "colour = \"red\"")).contains("colour"));
    }

    #[test]
    fn unknown_families_are_rejected() {
        let text = one("", "").replace("\"social\"", "\"hypergraph\"");
        assert!(err(&text).contains("hypergraph"));
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let text = one("", "") + &one("", "").replace("schema = 1", "");
        assert!(err(&text).contains("listed twice"));
    }

    #[test]
    fn malformed_names_and_reserved_selectors_are_rejected() {
        assert!(err(&one("", "").replace("\"toy\"", "\"Toy\"")).contains("name"));
        assert!(err(&one("", "").replace("\"toy\"", "\"tier1\"")).contains("reserved"));
        let bad_file = one("", "").replace("\"toy.txt\"", "\"../toy.txt\"");
        assert!(err(&bad_file).contains("plain name"));
    }

    #[test]
    fn checksums_must_be_lowercase_hex_of_the_right_length() {
        let upper = one("", "").replace(H64, &H64.replace('0', "A"));
        assert!(err(&upper).contains("sha256"));
        let short = one("", "").replace(H64, "abc");
        assert!(err(&short).contains("sha256"));
        assert!(err(&one("", "md5_published = \"xyz\"")).contains("md5_published"));
    }

    #[test]
    fn tiers_one_and_two_must_be_pinned_and_tier_three_must_say_so() {
        let unpinned = one("", "").replace(&format!("sha256 = \"{H64}\""), "");
        assert!(err(&unpinned).contains("required"));
        let tier3 = unpinned.replace("tier = 1", "tier = 3");
        assert!(err(&tier3).contains("unverified"));
        let declared = tier3.replace(
            "licence = \"none\"",
            "licence = \"none\"\nunverified = [\"sha256\"]",
        );
        Registry::parse(&declared).expect("tier 3 may be unpinned when it says so");
    }

    #[test]
    fn missing_counts_must_be_declared_unverified() {
        let no_nodes = one("", "").replace("nodes = 3", "");
        assert!(err(&no_nodes).contains("nodes"));
        let declared = no_nodes.replace(
            "licence = \"none\"",
            "licence = \"none\"\nunverified = [\"nodes\"]",
        );
        Registry::parse(&declared).expect("declared");
        assert!(err(&one("unverified = [\"colour\"]", "")).contains("colour"));
    }

    #[test]
    fn non_http_urls_are_rejected() {
        let ftp = one("", "").replace("https://example.org", "ftp://example.org");
        assert!(err(&ftp).contains("http"));
    }

    #[test]
    fn synthetic_datasets_need_a_generator_and_no_url() {
        let synth = one("", "").replace("\"social\"", "\"synthetic\"");
        assert!(err(&synth).contains("generator"));
        let with_gen = synth.replace(
            "licence = \"none\"",
            "licence = \"none\"\ngenerator = { kind = \"path\", n = 3 }",
        );
        assert!(err(&with_gen).contains("no URL"));
        let ok = with_gen.replace("url = \"https://example.org/toy.txt\"", "");
        Registry::parse(&ok).expect("a generated dataset");
        // A generated graph may stand for another family, but stays in tier 1.
        let weighted = ok.replace("\"synthetic\"", "\"weighted\"");
        Registry::parse(&weighted).expect("a generated weighted graph");
        assert!(err(&weighted.replace("tier = 1", "tier = 2")).contains("tier 1"));
    }

    #[test]
    fn a_wrong_schema_version_is_rejected() {
        assert!(err(&one("", "").replace("schema = 1", "schema = 2")).contains("schema"));
    }
}
