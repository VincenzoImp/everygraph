# Datasets

The graphs that Everygraph's benchmarks, quality gates and tests run on, registered by
name in [`datasets.toml`](datasets.toml) with where to get them, their size, SHA-256,
counts and terms of use.

**No dataset is redistributed in this repository.** Each entry points at the source that
publishes it; `everygraph-bench fetch` downloads it from there onto your machine. The
synthetic graphs are generated locally and are not data from anyone.

## Getting them

```sh
cargo build --release -p everygraph-bench
B=target/release/everygraph-bench

$B list                      # what is registered, and what is on disk and verified
$B fetch tier1               # the CI set: about 74 MB, plus the generated graphs
$B fetch com-livejournal     # one dataset by name
$B fetch tier2               # tiers 1 and 2: about 22 GB, mostly Friendster (SNAP and LDBC copies) and Twitter
$B verify tier1              # re-read the files and check them again
$B path roadnet-ca           # where the files of a dataset are
```

- **Data root.** `--data DIR`, else `$EVERYGRAPH_DATA`, else `./data` in the working
  directory (ignored by git, so a fetch from the repository root never adds data to the
  repository). Files go to `<root>/<dataset>/<file>`.
- **Tiers.** 1 = continuous integration (small; every family that CI needs);
  2 = workstation; 3 = server, optional. `tierN` selects that tier and the ones below.
- **Download.** Several ranged connections per file (`--connections`, default 8: SNAP
  serves about 0.7 MB/s per connection), resumed after an interruption from where each
  range stopped, checked against the size and SHA-256 in the registry (and against the MD5
  the source publishes, where it does), and moved into place only when it matches. A
  failed check deletes the download. A server that ignores `Range` gets one plain
  stream, refused up front if its declared length is wrong and cut one byte past the
  expected size. Placed files are made read-only.
- **A file that no longer matches** (the registry pinned a new version, or the file was
  edited) stops `fetch` with the path to delete; `fetch --replace` deletes it and fetches
  it again.
- **Local copies.** `--from-dir DIR` copies a file of the same name from `DIR` instead of
  downloading it, then checks it like a download. It is a copy, never a hard link, so
  the data root shares no inode with `DIR`; on file systems that support it the copy
  shares extents (a reflink, through `copy_file_range`). `DIR` is only read.
- **Unpinned files.** Tier 3 may hold files nobody has downloaded yet, so the registry
  has no SHA-256 for them. `fetch` refuses them unless given `--allow-unpinned`, and then
  reports the checksum to record in `datasets.toml`.

## Terms of use

Read the terms of each dataset before you use it; the table links them, and the sections
below quote them. The registry records what the source says, not legal advice. In short:

- **SNAP** (Stanford) states no licence for these datasets and asks that they be cited.
  `sx-stackoverflow` is derived from the Stack Exchange Data Dump (CC BY-SA).
- **LAW** (Milan) states no licence and asks to acknowledge WebGraph, LLP and the crawler
  that gathered each graph.
- **DIMACS10** `europe.osm` is derived from 2010 OpenStreetMap data.
- **Wikimedia** publishes the clickstream data under CC0.
- **No registered dataset states terms that forbid commercial use**, but SNAP and LAW
  state no terms at all, which is not the same as a licence. MovieLens 100K was in an
  earlier version of the registry as the bipartite graph and was removed: its terms
  allow research use only, with no redistribution. Tier 1 uses a generated bipartite
  graph instead.

Fields that could not be checked against the source are listed as "not verified" in each
section and marked † in the table. For most datasets that is the licence: a source that
states no licence cannot be checked for one.

## Adding a dataset

Add an entry to `datasets.toml` (the header comment lists the fields), fetch it once with
`--allow-unpinned` against a registry where it is tier 3 to learn its SHA-256, pin it, and
regenerate the tables below with
`everygraph-bench list --write-readme datasets/README.md`. A test fails if this file and
the registry disagree, and another if a family loses its last dataset.

## The registry

<!-- BEGIN GENERATED: everygraph-bench list --markdown -->

Values marked † are not verified against the source; see each dataset's notes.

| Name | Tier | Family | Dir. | Wt. | Time | Nodes | Edges | Download | Checksum | Licence / terms |
|---|---|---|---|---|---|--:|--:|--:|---|---|
| [`ego-facebook`](https://snap.stanford.edu/data/ego-Facebook.html) | 1 | social | no | no | no | 4,039 | 88,234 | 218.5 kB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/index.html) † |
| [`com-youtube`](https://snap.stanford.edu/data/com-Youtube.html) | 1 | social | no | no | no | 1,134,890 | 2,987,624 | 10.5 MB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/index.html) † |
| [`roadnet-ca`](https://snap.stanford.edu/data/roadNet-CA.html) | 1 | road | no | no | no | 1,965,206 | 2,766,607 | 17.8 MB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/index.html) † |
| [`web-google`](https://snap.stanford.edu/data/web-Google.html) | 1 | web | yes | no | no | 875,713 | 5,105,039 | 21.1 MB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/index.html) † |
| [`soc-epinions1`](https://snap.stanford.edu/data/soc-Epinions1.html) | 1 | directed-social | yes | no | no | 75,879 | 508,837 | 1.6 MB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/index.html) † |
| [`soc-sign-bitcoin-otc`](https://snap.stanford.edu/data/soc-sign-bitcoin-otc.html) | 1 | directed-social | yes | yes | yes | 5,881 | 35,592 | 397.2 kB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/soc-sign-bitcoin-otc.html) † |
| [`college-msg`](https://snap.stanford.edu/data/CollegeMsg.html) | 1 | temporal | yes | no | yes | 1,899 | 59,835 | 345.3 kB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/CollegeMsg.html) † |
| [`wiki-clickstream-ptwiki-2024-01`](https://meta.wikimedia.org/wiki/Research:Wikipedia_clickstream) | 1 | weighted | yes | yes | no | 288,605 † | 983,234 † | 22.1 MB | SHA-256 | [CC0 1.0 ("All Analytics datasets are available under the Creative Commons CC0 dedication", dumps.wikimedia.org)](https://dumps.wikimedia.org/other/clickstream/readme.html) |
| `synth-weighted-gnm-1000-5000` | 1 | weighted | no | yes | no | 1,000 | 5,000 | generated | SHA-256 | Generated by everygraph-bench; no third-party terms |
| `synth-bipartite-300x200` | 1 | bipartite | no | no | no | 500 | 3,000 | generated | SHA-256 | Generated by everygraph-bench; no third-party terms |
| `synth-path-100` | 1 | synthetic | no | no | no | 100 | 99 | generated | SHA-256 | Generated by everygraph-bench; no third-party terms |
| `synth-cycle-100` | 1 | synthetic | no | no | no | 100 | 100 | generated | SHA-256 | Generated by everygraph-bench; no third-party terms |
| `synth-star-100` | 1 | synthetic | no | no | no | 100 | 99 | generated | SHA-256 | Generated by everygraph-bench; no third-party terms |
| `synth-complete-20` | 1 | synthetic | no | no | no | 20 | 190 | generated | SHA-256 | Generated by everygraph-bench; no third-party terms |
| `synth-grid-30x30` | 1 | synthetic | no | no | no | 900 | 1,740 | generated | SHA-256 | Generated by everygraph-bench; no third-party terms |
| `synth-barbell-10` | 1 | synthetic | no | no | no | 20 | 91 | generated | SHA-256 | Generated by everygraph-bench; no third-party terms |
| `synth-gnm-1000-5000` | 1 | synthetic | no | no | no | 1,000 | 5,000 | generated | SHA-256 | Generated by everygraph-bench; no third-party terms |
| [`com-livejournal`](https://snap.stanford.edu/data/com-LiveJournal.html) | 2 | social | no | no | no | 3,997,962 | 34,681,189 | 124.2 MB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/index.html) † |
| [`com-orkut`](https://snap.stanford.edu/data/com-Orkut.html) | 2 | social | no | no | no | 3,072,441 | 117,185,083 | 447.2 MB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/index.html) † |
| [`com-friendster`](https://snap.stanford.edu/data/com-Friendster.html) | 2 | social | no | no | no | 65,608,366 | 1,806,067,135 | 9.3 GB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/index.html) † |
| [`twitter-2010`](https://law.di.unimi.it/webdata/twitter-2010/) | 2 | directed-social | yes | no | no | 41,652,230 | 1,468,365,182 | 2.5 GB | SHA-256 | [No licence stated by LAW](https://law.di.unimi.it/datasets.php) † |
| [`uk-2005`](https://law.di.unimi.it/webdata/uk-2005/) | 2 | web | yes | no | no | 39,459,925 | 936,364,282 | 210.0 MB | SHA-256 | [No licence stated by LAW](https://law.di.unimi.it/datasets.php) † |
| [`europe-osm`](https://sites.cc.gatech.edu/dimacs10/archive/streets.shtml) | 2 | road | no | no | no | 50,912,018 | 54,054,660 | 258.3 MB | SHA-256 | [Derived from OpenStreetMap data (Geofabrik extract, March 2010)](https://www.openstreetmap.org/copyright) † |
| [`sx-stackoverflow`](https://snap.stanford.edu/data/sx-stackoverflow.html) | 2 | temporal | yes | no | yes | 2,601,977 | 63,497,050 | 529.8 MB | SHA-256 | [No licence stated by SNAP for the graph](https://archive.org/details/stackexchange) † |
| [`ldbc-test`](https://ldbcouncil.org/benchmarks/graphalytics/datasets/) | 1 | conformance | yes | yes | no | 212 | 523 | 10.1 kB | SHA-256 | [No licence stated by LDBC for the data sets (checked on the data-sets page, 2026-11-26)](https://ldbcouncil.org/benchmarks/fair-use-policies/) † |
| [`ldbc-graph500-22`](https://ldbcouncil.org/benchmarks/graphalytics/datasets/) | 2 | conformance | no | no | no | 2,396,657 | 64,155,735 | 212.2 MB | SHA-256 | [No licence stated by LDBC for the data sets (checked on the data-sets page, 2026-11-26)](https://ldbcouncil.org/benchmarks/fair-use-policies/) † |
| [`ldbc-dota-league`](https://ldbcouncil.org/benchmarks/graphalytics/datasets/) | 2 | conformance | no | yes | no | 61,170 | 50,870,313 | 119.8 MB | SHA-256 | [No licence stated by LDBC for the data sets (checked on the data-sets page, 2026-11-26)](https://ldbcouncil.org/benchmarks/fair-use-policies/) † |
| [`ldbc-com-friendster`](https://ldbcouncil.org/benchmarks/graphalytics/datasets/) | 2 | conformance | no | no | no | 65,608,366 | 1,806,067,135 | 7.2 GB | SHA-256 | [No licence stated by LDBC for the data sets (checked on the data-sets page, 2026-11-26)](https://ldbcouncil.org/benchmarks/fair-use-policies/) † |
| [`gsh-2015`](https://law.di.unimi.it/webdata/gsh-2015/) | 3 | web | yes | no | no | 988,490,691 | 33,877,399,152 | 8.6 GB | MD5 (source) only † | [No licence stated by LAW](https://law.di.unimi.it/datasets.php) † |

### `ego-facebook`

Facebook ego networks, all ten combined (SNAP ego-Facebook, facebook_combined)

- **Format:** Edge list, gzip: one 'u v' pair per line (space-separated), no header
- **To an edge list:** gunzip. Each line is one undirected edge, listed once. Ids are 0-based and contiguous.
- **Counts:** Undirected edges. Checked on the file: 88,234 lines, 4,039 distinct ids (0..4038).
- **Licence / terms:** No licence stated by SNAP. SNAP asks that the dataset be cited.
- **Cite:** J. McAuley and J. Leskovec. Learning to Discover Social Circles in Ego Networks. NIPS, 2012.
- **Notes:** The smallest real graph of the registry; used by the ignored-by-default integration test.
- **Not verified:** licence
- **File:** [`facebook_combined.txt.gz`](https://snap.stanford.edu/data/facebook_combined.txt.gz), 218576 bytes, SHA-256 `125e84db872eeba443d270c70315c256b0af43a502fcfe51f50621166ad035d7`

### `com-youtube`

YouTube friendship network with ground-truth communities (SNAP com-Youtube)

- **Format:** SNAP edge list, gzip: '#' comment header, then 'u\tv' per line
- **To an edge list:** gunzip, drop lines starting with '#'. Each undirected edge is listed once. Ids are not contiguous (largest id 1,157,827 for 1,134,890 nodes): remap.
- **Counts:** Undirected edges. Checked on the file: 2,987,624 edge lines; the header repeats both counts.
- **Licence / terms:** No licence stated by SNAP. SNAP asks that the dataset be cited.
- **Cite:** J. Yang and J. Leskovec. Defining and Evaluating Network Communities based on Ground-truth. ICDM, 2012.
- **Notes:** The tier-1 social graph at scale (3 M edges). Ground-truth communities are published separately on the same page (not registered).
- **Not verified:** licence
- **File:** [`com-youtube.ungraph.txt.gz`](https://snap.stanford.edu/data/bigdata/communities/com-youtube.ungraph.txt.gz), 10590328 bytes, SHA-256 `dff1b97ba7d2fa9c59884b67dcd2275e717ff9501f86ed82ce6582ed4971f3e0`

### `roadnet-ca`

California road network (SNAP roadNet-CA)

- **Format:** SNAP edge list, gzip, CRLF line endings: '#' header, then 'u\tv' per line
- **To an edge list:** gunzip, drop '#' lines, strip the trailing '\r'. Every undirected edge appears as u->v and v->u: keep u < v to get each once. Ids are not contiguous (largest 1,971,280): remap.
- **Counts:** The page counts 2,766,607 undirected edges; the file holds each in both directions: 5,533,214 lines, as its header says. 1,965,206 distinct ids, checked.
- **Licence / terms:** No licence stated by SNAP. SNAP asks that the dataset be cited.
- **Cite:** J. Leskovec, K. Lang, A. Dasgupta, M. Mahoney. Community Structure in Large Networks: Natural Cluster Sizes and the Absence of Large Well-Defined Clusters. Internet Mathematics 6(1) 29-123, 2009.
- **Notes:** High diameter, degree about 2 to 3: the weak case of direction-optimizing BFS (issue #25).
- **Not verified:** licence
- **File:** [`roadNet-CA.txt.gz`](https://snap.stanford.edu/data/roadNet-CA.txt.gz), 17892860 bytes, SHA-256 `383f7b14424530a9e25c392969a9c19dace32ebcfd141f0967a1433498c2acdd`

### `web-google`

Web graph from the Google programming contest, 2002 (SNAP web-Google)

- **Format:** SNAP edge list, gzip, CRLF line endings: '#' header, then 'from\tto' per line
- **To an edge list:** gunzip, drop '#' lines, strip the trailing '\r'. One directed hyperlink per line. Ids are not contiguous (largest 916,427): remap.
- **Counts:** Directed edges. Checked on the file: 5,105,039 edge lines, 875,713 distinct ids.
- **Licence / terms:** No licence stated by SNAP. SNAP asks that the dataset be cited.
- **Cite:** J. Leskovec, K. Lang, A. Dasgupta, M. Mahoney. Community Structure in Large Networks: Natural Cluster Sizes and the Absence of Large Well-Defined Clusters. Internet Mathematics 6(1) 29-123, 2009.
- **Not verified:** licence
- **File:** [`web-Google.txt.gz`](https://snap.stanford.edu/data/web-Google.txt.gz), 21168784 bytes, SHA-256 `bcac0af0471d749f4a8c010bca92b61cf2868a0570741de06892fc062f265ea6`

### `soc-epinions1`

Epinions who-trusts-whom network (SNAP soc-Epinions1)

- **Format:** SNAP edge list, gzip, CRLF line endings: '#' header, then 'from\tto' per line
- **To an edge list:** gunzip, drop '#' lines, strip the trailing '\r'. One directed trust edge per line. Ids are 0-based with gaps (largest 75,887).
- **Counts:** Directed edges. Checked on the file: 508,837 edge lines, 75,879 distinct ids.
- **Licence / terms:** No licence stated by SNAP. SNAP asks that the dataset be cited.
- **Cite:** M. Richardson, R. Agrawal and P. Domingos. Trust Management for the Semantic Web. ISWC, 2003.
- **Not verified:** licence
- **File:** [`soc-Epinions1.txt.gz`](https://snap.stanford.edu/data/soc-Epinions1.txt.gz), 1630437 bytes, SHA-256 `69a2dab71fa5e3a0715487599fc16ca17ddc847379325a6c765bbad6e3e36938`

### `soc-sign-bitcoin-otc`

Bitcoin OTC trust network, weighted and signed, with timestamps (SNAP soc-sign-bitcoin-otc)

- **Format:** CSV, gzip, no header: SOURCE,TARGET,RATING,TIME (TIME in seconds since the epoch, with a fraction)
- **To an edge list:** gunzip; columns 1 and 2 are the edge, column 3 the weight (an integer in -10..+10), column 4 the time.
- **Counts:** Directed rated edges. Checked on the file: 35,592 lines, 5,881 distinct ids, weights from -10 to +10.
- **Licence / terms:** No licence stated by SNAP. SNAP asks that the papers on the page be cited.
- **Cite:** S. Kumar, F. Spezzano, V.S. Subrahmanian, C. Faloutsos. Edge Weight Prediction in Weighted Signed Networks. ICDM, 2016.
- **Notes:** Weights are signed (-10 to +10), so it does not stand for the weighted family: it is here to test how kernels that need positive weights (Leiden modularity, Dijkstra) refuse or handle negative ones. The weighted family is wiki-clickstream-ptwiki-2024-01 and the generated synth-weighted-gnm-1000-5000.
- **Not verified:** licence
- **File:** [`soc-sign-bitcoinotc.csv.gz`](https://snap.stanford.edu/data/soc-sign-bitcoinotc.csv.gz), 397258 bytes, SHA-256 `6424ac981dad3a019f697fc1b9fcd85c19d8d9f039797758e9ffb6fea100c373`

### `college-msg`

Private messages of a college online community, with timestamps (SNAP CollegeMsg)

- **Format:** Edge list, gzip, no header: 'SRC TGT UNIXTS' per line (space-separated)
- **To an edge list:** gunzip. Each line is one message u -> v at time t; parallel edges are real. Ids are 1-based and contiguous.
- **Counts:** Edges are temporal edges (messages), a multigraph; SNAP also lists 20,296 edges in the static graph. Checked on the file: 59,835 lines, 1,899 distinct ids (1..1899).
- **Licence / terms:** No licence stated by SNAP. SNAP asks that the dataset be cited.
- **Cite:** P. Panzarasa, T. Opsahl, K. M. Carley. Patterns and dynamics of users' behavior and interaction: Network analysis of an online community. JASIST 60(5), 2009.
- **Not verified:** licence
- **File:** [`CollegeMsg.txt.gz`](https://snap.stanford.edu/data/CollegeMsg.txt.gz), 345339 bytes, SHA-256 `50ae2d98ed3bad9ddb18dbd495a89e5e10cfb8f7e86932827db29fc41b41f9fa`

### `wiki-clickstream-ptwiki-2024-01`

Portuguese Wikipedia clickstream, January 2024: article-to-article clicks with counts (Wikimedia clickstream-ptwiki-2024-01)

- **Format:** TSV, gzip, no header: 'prev\tcurr\ttype\tn' per line; prev and curr are article titles, type is link, external or other, n the number of clicks
- **To an edge list:** gunzip; keep rows whose type is 'link' (both ends are articles and prev links to curr); prev -> curr is the edge, n its positive integer weight. Titles are strings: build an id dictionary.
- **Counts:** Not published by the source: counted on the file, keeping only rows of type 'link' (983,234 of 1,763,469 rows, all distinct (prev, curr) pairs, 288,605 distinct titles). Weights are click counts, the smallest 10.
- **Licence / terms:** CC0 1.0 ("All Analytics datasets are available under the Creative Commons CC0 dedication", dumps.wikimedia.org).
- **Cite:** Wikimedia Foundation, Wikipedia clickstream (https://meta.wikimedia.org/wiki/Research:Wikipedia_clickstream). CC0 asks for no citation.
- **Notes:** The real positively weighted graph of tier 1: directed, weights are counts (a clicks-between-articles graph rather than a co-occurrence graph). The meta page says pairs with 10 or fewer observations are removed; the smallest count in this file is 10. The January 2024 file was still online on 2026-11-25; no promise of permanence was found.
- **Not verified:** nodes, edges
- **File:** [`clickstream-ptwiki-2024-01.tsv.gz`](https://dumps.wikimedia.org/other/clickstream/2024-01/clickstream-ptwiki-2024-01.tsv.gz), 22182571 bytes, SHA-256 `223441b104c0fd0488beaeaf73fc41dba4cbee3cf25052bac91d460a6d02b2fb`

### `synth-weighted-gnm-1000-5000`

Weighted G(n, m) random graph: 1,000 nodes, 5,000 distinct edges, integer weights 1..10, seed 7

- **Format:** Edge list: '#' header, then 'u\tv\tw' with u < v and w in 1..10
- **To an edge list:** Drop '#' lines; the third column is the weight.
- **Licence / terms:** Generated by everygraph-bench; no third-party terms.
- **Notes:** The weighted graph CI can always rely on: positive integer weights, no network, no terms.
- **File:** `weighted-gnm-1000-5000.txt` (generated), 49516 bytes, SHA-256 `982d40e2c157a55673978882050603a0a117bfab61c753d8720968c923f3db9c`

### `synth-bipartite-300x200`

Random bipartite graph: 300 + 200 nodes, 3,000 distinct edges, seed 11

- **Format:** Edge list: '#' header, then 'u\tv' with u in 0..300 (left) and v in 300..500 (right)
- **To an edge list:** Drop '#' lines. The two sides share one id space: left 0..299, right 300..499.
- **Licence / terms:** Generated by everygraph-bench; no third-party terms.
- **Notes:** The bipartite graph of tier 1. No real bipartite graph with permissive terms is registered yet (MovieLens 100K was removed: its terms forbid commercial use and redistribution).
- **File:** `bipartite-300x200.txt` (generated), 22996 bytes, SHA-256 `8f787a9414cd72ab030c20368ee91c73b903bf81712fa0b3a80358dd818ea2a3`

### `synth-path-100`

Path on 100 nodes

- **Format:** Edge list: '#' header, then 'u\tv' with u < v
- **To an edge list:** Drop '#' lines.
- **Licence / terms:** Generated by everygraph-bench; no third-party terms.
- **File:** `path-100.txt` (generated), 637 bytes, SHA-256 `98a28a9f363bb30939f32e608d0426eaf11258e2d5e13578e280c22f9aa0d75e`

### `synth-cycle-100`

Cycle on 100 nodes

- **Format:** Edge list: '#' header, then 'u\tv' with u < v
- **To an edge list:** Drop '#' lines.
- **Licence / terms:** Generated by everygraph-bench; no third-party terms.
- **File:** `cycle-100.txt` (generated), 644 bytes, SHA-256 `a207dea49f788c1fb9acdc7f5353bd0217731450b17a67cdf0503f956281721f`

### `synth-star-100`

Star with one centre and 99 leaves

- **Format:** Edge list: '#' header, then 'u\tv' with u < v
- **To an edge list:** Drop '#' lines.
- **Licence / terms:** Generated by everygraph-bench; no third-party terms.
- **File:** `star-100.txt` (generated), 548 bytes, SHA-256 `6afb08442ee28411b116d013a181e5963260727226717f820e80f404168d1e3b`

### `synth-complete-20`

Complete graph on 20 nodes

- **Format:** Edge list: '#' header, then 'u\tv' with u < v
- **To an edge list:** Drop '#' lines.
- **Licence / terms:** Generated by everygraph-bench; no third-party terms.
- **File:** `complete-20.txt` (generated), 1015 bytes, SHA-256 `f62b96e6ee17a065036e5b4ef5a814f85b60ef58ce899a86222e5fbce68b24bf`

### `synth-grid-30x30`

30 x 30 grid with 4-neighbourhoods

- **Format:** Edge list: '#' header, then 'u\tv' with u < v
- **To an edge list:** Drop '#' lines. Node r * 30 + c is row r, column c.
- **Licence / terms:** Generated by everygraph-bench; no third-party terms.
- **File:** `grid-30x30.txt` (generated), 13602 bytes, SHA-256 `c603a1dbf0b8ea11c281adc578ee0310cd06d3b3e04f80457ddb17ce4bf39232`

### `synth-barbell-10`

Two 10-cliques joined by one edge

- **Format:** Edge list: '#' header, then 'u\tv' with u < v
- **To an edge list:** Drop '#' lines. Nodes 0..9 and 10..19 are the cliques; 9-10 is the bridge.
- **Licence / terms:** Generated by everygraph-bench; no third-party terms.
- **Notes:** Two obvious communities: a sanity check for community detection.
- **File:** `barbell-10.txt` (generated), 518 bytes, SHA-256 `2e108e28fcb6b9a280b227b434caa62bd0a3700f905b6547203d1cf7b2102e34`

### `synth-gnm-1000-5000`

G(n, m) random graph: 1,000 nodes, 5,000 distinct edges, seed 42

- **Format:** Edge list: '#' header, then 'u\tv' with u < v
- **To an edge list:** Drop '#' lines. Edges are in generation order, not sorted.
- **Licence / terms:** Generated by everygraph-bench; no third-party terms.
- **Notes:** SplitMix64 stream, so the bytes do not depend on any crate's random-number implementation. Some nodes may be isolated.
- **File:** `gnm-1000-5000.txt` (generated), 38966 bytes, SHA-256 `cf084616d259d6c5ea6f47a075524e158edd058cc8208d4c6be747813680a020`

### `com-livejournal`

LiveJournal friendship network with ground-truth communities (SNAP com-LiveJournal)

- **Format:** SNAP edge list, gzip: '#' comment header, then 'u\tv' per line
- **To an edge list:** gunzip, drop '#' lines. Each undirected edge is listed once. Ids are not contiguous: remap.
- **Counts:** Undirected edges, as published on the page.
- **Licence / terms:** No licence stated by SNAP. SNAP asks that the dataset be cited.
- **Cite:** J. Yang and J. Leskovec. Defining and Evaluating Network Communities based on Ground-truth. ICDM, 2012.
- **Notes:** The graph of the first lab's headline numbers (docs/FINDINGS.md). Issue #5 listed it in tier 1; it is tier 2 here because its 124 MB are twice the rest of tier 1 together and CI needs no graph this large.
- **Not verified:** licence
- **File:** [`com-lj.ungraph.txt.gz`](https://snap.stanford.edu/data/bigdata/communities/com-lj.ungraph.txt.gz), 124262769 bytes, SHA-256 `e0e8996341b963ecbe0a572c71446c03fd6e93a6c521578eaae7566c68ff608e`

### `com-orkut`

Orkut friendship network with ground-truth communities (SNAP com-Orkut)

- **Format:** SNAP edge list, gzip: '#' comment header, then 'u\tv' per line
- **To an edge list:** gunzip, drop '#' lines. Each undirected edge is listed once. Ids are not contiguous: remap.
- **Counts:** Undirected edges, as published on the page.
- **Licence / terms:** No licence stated by SNAP. SNAP asks that the dataset be cited.
- **Cite:** J. Yang and J. Leskovec. Defining and Evaluating Network Communities based on Ground-truth. ICDM, 2012.
- **Notes:** SHA-256 computed from the lab's copy (/home/tux/graphlab/data/raw, downloaded 2026-11-24), whose size equals the server's Content-Length; SNAP publishes no checksum to compare with.
- **Not verified:** licence
- **File:** [`com-orkut.ungraph.txt.gz`](https://snap.stanford.edu/data/bigdata/communities/com-orkut.ungraph.txt.gz), 447251958 bytes, SHA-256 `f73e33fb685f411a10c952f2ba3ea788380b91a17bc636e38da1a23f6c6b2bc6`

### `com-friendster`

Friendster friendship network with ground-truth communities (SNAP com-Friendster)

- **Format:** SNAP edge list, gzip: '#' comment header, then 'u\tv' per line
- **To an edge list:** gunzip, drop '#' lines. Each undirected edge is listed once. Ids are not contiguous: remap.
- **Counts:** Undirected edges, as published on the page.
- **Licence / terms:** No licence stated by SNAP. SNAP asks that the dataset be cited.
- **Cite:** J. Yang and J. Leskovec. Defining and Evaluating Network Communities based on Ground-truth. ICDM, 2012.
- **Notes:** 9.4 GB compressed; at SNAP's ~0.7 MB/s per connection a single stream takes hours. SHA-256 computed from the lab's copy (/home/tux/graphlab/data/raw, downloaded 2026-11-24), whose size equals the server's Content-Length; it was not downloaded again, and SNAP publishes no checksum to compare with.
- **Not verified:** licence
- **File:** [`com-friendster.ungraph.txt.gz`](https://snap.stanford.edu/data/bigdata/communities/com-friendster.ungraph.txt.gz), 9371322431 bytes, SHA-256 `2efc60acf958220467b0bfc4ffcd2be9f46804dbfd9899e9796204d0ee41a5c6`

### `twitter-2010`

Twitter follower graph, 2010 crawl (LAW twitter-2010)

- **Format:** WebGraph BVGraph (.graph + .properties); the .offsets file is not published and is rebuilt by WebGraph
- **To an edge list:** Needs a WebGraph reader: the optional webgraph feature of issue #42 (webgraph-rs), or the Java WebGraph tools (for example ArcListASCIIGraph) to write an arc list.
- **Counts:** nodes and arcs from twitter-2010.properties.
- **Licence / terms:** No licence stated by LAW. LAW asks to acknowledge WebGraph and LLP when publishing results; the crawl is by Kwak et al.
- **Cite:** H. Kwak, C. Lee, H. Park, S. Moon. What is Twitter, a social network or a news media? WWW 2010. Plus WebGraph (Boldi, Vigna, WWW 2004) and LLP (Boldi, Rosa, Santini, Vigna, WWW 2011).
- **Notes:** Reading it needs the optional webgraph feature (issue #42). SHA-256 computed from the lab's copy (/home/tux/graphlab/data/raw); its MD5 equals the one LAW publishes in twitter-2010.md5sums, for both files. The direction of an arc (follower to followed or the reverse) is as described by LAW; not checked here.
- **Not verified:** licence
- **File:** [`twitter-2010.graph`](http://data.law.di.unimi.it/webdata/twitter-2010/twitter-2010.graph), 2591013817 bytes, SHA-256 `e29ef53ba00f9dd441e142454745f4bb1c4274d19815b2983b5d7e640f9e58bb`
- **File:** [`twitter-2010.properties`](http://data.law.di.unimi.it/webdata/twitter-2010/twitter-2010.properties), 1272 bytes, SHA-256 `79fa8752cc95cc1e9ae7aec455c0076d490d5e9f9b883262fa7d357e0076543c`

### `uk-2005`

Web crawl of the .uk domain, 2005 (LAW uk-2005)

- **Format:** WebGraph BVGraph (.graph + .properties); the .offsets file is not published and is rebuilt by WebGraph
- **To an edge list:** Needs a WebGraph reader: the optional webgraph feature of issue #42 (webgraph-rs), or the Java WebGraph tools to write an arc list.
- **Counts:** nodes and arcs from uk-2005.properties.
- **Licence / terms:** No licence stated by LAW. LAW asks to acknowledge WebGraph, LLP and UbiCrawler when publishing results.
- **Cite:** WebGraph (Boldi, Vigna, WWW 2004), LLP (Boldi, Rosa, Santini, Vigna, WWW 2011), UbiCrawler (Boldi, Codenotti, Santini, Vigna, 2004).
- **Notes:** Reading it needs the optional webgraph feature (issue #42). Downloaded fresh; MD5 equals the one LAW publishes in uk-2005.md5sums, for both files.
- **Not verified:** licence
- **File:** [`uk-2005.graph`](http://data.law.di.unimi.it/webdata/uk-2005/uk-2005.graph), 209998792 bytes, SHA-256 `a73fff765e2bc1c34def5162a28831781a2c7bba7beb4472d54bd15a923b5fad`
- **File:** [`uk-2005.properties`](http://data.law.di.unimi.it/webdata/uk-2005/uk-2005.properties), 1211 bytes, SHA-256 `ff495767649fe1c00caeebca55eef506c7d6e94b4aa7aada7d7e400979c1a900`

### `europe-osm`

European road network from OpenStreetMap, largest strongly connected component (DIMACS10 europe.osm)

- **Format:** METIS graph, bzip2: '%' comment lines, a line 'n m', then line i lists the neighbours of node i (1-based)
- **To an edge list:** bunzip2, skip '%' lines and the 'n m' line; for line i (from 1) emit (i, j) for each neighbour j > i. Coordinates are in europe.osm.xyz.bz2 on the same page (not registered).
- **Counts:** Undirected edges, from the DIMACS10 street-networks page; the file's first line says '50912018 54054660'.
- **Licence / terms:** Derived from OpenStreetMap data (Geofabrik extract, March 2010). OpenStreetMap data of that date was published under CC BY-SA 2.0 (OpenStreetMap moved to the ODbL 1.0 in 2012); the DIMACS10 page states no terms of its own.
- **Cite:** D. A. Bader, H. Meyerhenke, P. Sanders, D. Wagner (eds.). Graph Partitioning and Graph Clustering. 10th DIMACS Implementation Challenge, Contemporary Mathematics 588, 2013. Map data (c) OpenStreetMap contributors.
- **Notes:** The SuiteSparse copy (DIMACS10/europe_osm) is unreachable: its file host has answered 403 to every request since mid-October 2026 (the collection's site says it is offline because of excess downloads). This entry uses the original DIMACS10 file at Georgia Tech instead: the same graph in METIS rather than Matrix Market format. Which OpenStreetMap terms bind this 2010 extract today was not established.
- **Not verified:** licence
- **File:** [`europe.osm.graph.bz2`](https://sites.cc.gatech.edu/dimacs10/archive/data/streets/europe.osm.graph.bz2), 258310855 bytes, SHA-256 `5c170df84744bbbad46a51ce39dd47416f4fd2ea2b829f44a6917ee6c6e0a3c6`

### `sx-stackoverflow`

Stack Overflow interactions (answers and comments) with timestamps (SNAP sx-stackoverflow)

- **Format:** Edge list, gzip, no header: 'SRC TGT UNIXTS' per line (space-separated)
- **To an edge list:** gunzip. Each line is one interaction u -> v at time t (u answered or commented on v's post); parallel edges are real. Ids are Stack Overflow user ids: remap.
- **Counts:** Edges are temporal edges; SNAP also lists 36,233,450 edges in the static graph, over 2,774 days. Checked on the file: 63,497,050 lines.
- **Licence / terms:** No licence stated by SNAP for the graph. It is derived from the Stack Exchange Data Dump, whose archive.org page gives the licence as CC BY-SA 4.0.
- **Cite:** A. Paranjape, A. R. Benson, J. Leskovec. Motifs in Temporal Networks. WSDM, 2017.
- **Notes:** The large transaction-style edge list of the registry: a timestamped interaction multigraph. Whether CC BY-SA carries over to the derived graph was not established.
- **Not verified:** licence
- **File:** [`sx-stackoverflow.txt.gz`](https://snap.stanford.edu/data/sx-stackoverflow.txt.gz), 529869668 bytes, SHA-256 `fc04264de1652bbf91c10eb8ec2465df082fc0aec2012e5f4a8edc6400f0d7a3`

### `ldbc-test`

LDBC Graphalytics test graphs: example-directed, example-undirected and test-<algorithm>-<directed|undirected>

- **Format:** tar.zst per graph: <g>.properties, <g>.v (one vertex id per line), <g>.e ('u v' or 'u v w', space-separated), <g>-BFS, -CDLP, -LCC, -PR, -SSSP, -WCC ('id value' per line)
- **To an edge list:** unzstd, untar. <g>.e is the edge list; vertex ids are listed in <g>.v and are not contiguous. Undirected graphs list each edge once.
- **Counts:** Summed over the fourteen graphs (at most 50 vertices each), from their .properties files and equal to what the reader counted. directed and weighted are true for some of them: each graph's .properties file says which.
- **Licence / terms:** No licence stated by LDBC for the data sets (checked on the data-sets page, 2026-11-26). The LDBC fair-use policy applies to results: unaudited numbers must not be called LDBC Benchmark Results.
- **Cite:** A. Iosup et al. The LDBC Graphalytics Benchmark. arXiv 2011.15028, v6, 2023.
- **Notes:** The test-* archives were updated on 2023-03-05 (Last-Modified), after the reference-output corrections of March 2023.
- **Not verified:** licence
- **File:** [`example-directed.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/example-directed.tar.zst), 1104 bytes, SHA-256 `30e0dc9984fcd4334f88eef0f419c8f9d2ff3333403c0d49e2ea062f5c4891b4`
- **File:** [`example-undirected.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/example-undirected.tar.zst), 1066 bytes, SHA-256 `69eb344d0d8438b6ee971378da3a2b3c623e0930393e2d5777e194091c3999ec`
- **File:** [`test-bfs-directed.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-bfs-directed.tar.zst), 504 bytes, SHA-256 `70353ffe8aa953edf85d81e238089cfec808d1a2822a69293c7a7493f4c46916`
- **File:** [`test-bfs-undirected.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-bfs-undirected.tar.zst), 504 bytes, SHA-256 `27c9748ab5c624c64aed3bf48fd937aaf67d480f676c6a4ff41b3822b0ed84f8`
- **File:** [`test-cdlp-directed.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-cdlp-directed.tar.zst), 482 bytes, SHA-256 `d4d068edefa596dfd13074a8c15baa06d82009240ed42d9af5b260d88c1652d6`
- **File:** [`test-cdlp-undirected.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-cdlp-undirected.tar.zst), 475 bytes, SHA-256 `0b8bdaed6bda7ecfa5c0d1018192e44dc7e1306f1c9bb20ffcd4a55ba57bc663`
- **File:** [`test-pr-directed.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-pr-directed.tar.zst), 1635 bytes, SHA-256 `25dbd25c1bf0d9dc55ebdeb4d45299ccacc27c91feccc3e1d448ea8b82342374`
- **File:** [`test-pr-undirected.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-pr-undirected.tar.zst), 1380 bytes, SHA-256 `49a67b0c1978f47f552f593d7ccc0b2e1bac383e29736efeb43d7e9aba74a9a0`
- **File:** [`test-lcc-directed.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-lcc-directed.tar.zst), 490 bytes, SHA-256 `523ddca56a5d859c543e8f967c265b3b28637e8c86a0abb44a6a12bf8795a2c1`
- **File:** [`test-lcc-undirected.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-lcc-undirected.tar.zst), 468 bytes, SHA-256 `c3b8ec5ea1a3f55382a10b23040d1dbf4e626cf8b96439f46d3d1d3f10a43710`
- **File:** [`test-wcc-directed.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-wcc-directed.tar.zst), 433 bytes, SHA-256 `14d5f462fa9e77823756fc87f4fc85c3d906a4ebaa0ebecac0758ac07819541b`
- **File:** [`test-wcc-undirected.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-wcc-undirected.tar.zst), 428 bytes, SHA-256 `c3d5f73b02a4a71db55ef0bedea9618dbb01f2429cf7068b1b7e741b00890d26`
- **File:** [`test-sssp-directed.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-sssp-directed.tar.zst), 603 bytes, SHA-256 `97b6b72184e6137af271dd2f3088aa68452510e08b3cea98da860a378bfa9bb6`
- **File:** [`test-sssp-undirected.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/test-sssp-undirected.tar.zst), 625 bytes, SHA-256 `908f7b9c12c4a7b3fb0836547b3cd49543448999ed450c575ede88f1149d7306`

### `ldbc-graph500-22`

LDBC Graphalytics graph500-22: Graph500 Kronecker generator, scale 22

- **Format:** tar.zst: graph500-22.properties, graph500-22.v (one vertex id per line), graph500-22.e ('u v' or 'u v w', space-separated), graph500-22-BFS, -CDLP, -LCC, -PR, -SSSP, -WCC ('id value' per line)
- **To an edge list:** unzstd, untar. graph500-22.e is the edge list, each undirected edge listed once; vertex ids are listed in graph500-22.v.
- **Counts:** From the .properties file (meta.vertices, meta.edges), and equal to what the oracle's LDBC reader counted on the .v and .e files.
- **Licence / terms:** No licence stated by LDBC for the data sets (checked on the data-sets page, 2026-11-26). The LDBC fair-use policy applies to results: unaudited numbers must not be called LDBC Benchmark Results.
- **Cite:** A. Iosup et al. The LDBC Graphalytics Benchmark. arXiv 2011.15028, v6, 2023.
- **Notes:** Synthetic (Kronecker, Graph500 scale 22), generated by LDBC and published with reference outputs. Undirected. The .v file lists 2,396,657 of the 2^22 = 4,194,304 vertex ids of scale 22: vertices without edges are not in the data set.
- **Not verified:** licence
- **File:** [`graph500-22.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/graph500-22.tar.zst), 212233822 bytes, SHA-256 `d88e1cb7ca83a2348a9a90d6796991c0a780ed1e3882bd85e36c50f3346ce074`

### `ldbc-dota-league`

LDBC Graphalytics dota-league: Dota 2 league players, edge weight = games played together

- **Format:** tar.zst: dota-league.properties, dota-league.v (one vertex id per line), dota-league.e ('u v' or 'u v w', space-separated), dota-league-BFS, -CDLP, -LCC, -PR, -SSSP, -WCC ('id value' per line)
- **To an edge list:** unzstd, untar. dota-league.e is the edge list, each undirected edge listed once; vertex ids are listed in dota-league.v.
- **Counts:** From the .properties file (meta.vertices, meta.edges), and equal to what the oracle's LDBC reader counted on the .v and .e files.
- **Licence / terms:** No licence stated by LDBC for the data sets (checked on the data-sets page, 2026-11-26). The LDBC fair-use policy applies to results: unaudited numbers must not be called LDBC Benchmark Results.
- **Cite:** A. Iosup et al. The LDBC Graphalytics Benchmark. arXiv 2011.15028, v6, 2023.
- **Notes:** A real weighted graph from the Game Trace Archive, as packaged by LDBC with reference outputs.
- **Not verified:** licence
- **File:** [`dota-league.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/dota-league.tar.zst), 119883349 bytes, SHA-256 `d83a92574e177b47fbcab951c8c0e1be3bb5674a18f3e7179f772bc60e72cb2d`

### `ldbc-com-friendster`

LDBC Graphalytics com-friendster: SNAP com-Friendster, as packaged by LDBC with reference outputs

- **Format:** tar.zst: com-friendster.properties, com-friendster.v (one vertex id per line), com-friendster.e ('u v' or 'u v w', space-separated), com-friendster-BFS, -CDLP, -LCC, -PR, -SSSP, -WCC ('id value' per line)
- **To an edge list:** unzstd, untar. com-friendster.e is the edge list, each undirected edge listed once; vertex ids are listed in com-friendster.v.
- **Counts:** From the .properties file (meta.vertices, meta.edges), and equal to what the oracle's LDBC reader counted on the .v and .e files.
- **Licence / terms:** No licence stated by LDBC for the data sets (checked on the data-sets page, 2026-11-26). The LDBC fair-use policy applies to results: unaudited numbers must not be called LDBC Benchmark Results.
- **Cite:** A. Iosup et al. The LDBC Graphalytics Benchmark. arXiv 2011.15028, v6, 2023.
- **Notes:** The same graph as the `com-friendster` entry (SNAP), with LDBC's vertex file and reference outputs. SNAP terms also apply: SNAP asks that the dataset be cited.
- **Not verified:** licence
- **File:** [`com-friendster.tar.zst`](https://datasets.ldbcouncil.org/graphalytics/com-friendster.tar.zst), 7237160329 bytes, SHA-256 `f18cc17419453c68bac5174426ce5ab45c5d6973646c92651553c53b08638e03`

### `gsh-2015`

General host-level-sampled web crawl, 2015 (LAW gsh-2015)

- **Format:** WebGraph BVGraph (.graph + .properties); the .offsets file is not published and is rebuilt by WebGraph
- **To an edge list:** Needs a WebGraph reader: the optional webgraph feature of issue #42 (webgraph-rs). An arc list would be hundreds of GB: process it in BVGraph form.
- **Counts:** nodes and arcs from gsh-2015.properties: about 34 billion arcs.
- **Licence / terms:** No licence stated by LAW. LAW asks to acknowledge WebGraph, LLP and BUbiNG when publishing results.
- **Cite:** WebGraph (Boldi, Vigna, WWW 2004), LLP (Boldi, Rosa, Santini, Vigna, WWW 2011), BUbiNG (Boldi, Marino, Santini, Vigna, 2018).
- **Notes:** Not downloaded (8.7 GB): SHA-256 is not pinned, so `fetch` needs --allow-unpinned and checks the MD5 that LAW publishes. Pin the SHA-256 it reports. LAW graphs above 10 billion arcs, as listed on 2026-11-25: uk-2014 (47.6 B arcs, 7.8 GB), gsh-2015 (33.9 B, 8.7 GB), clueweb12 (42.6 B, 11.9 GB), eu-2015 (91.8 B, 13.7 GB). gsh-2015 is chosen as a crawl not limited to one country domain; uk-2014 is the smaller download.
- **Not verified:** licence, sha256
- **File:** [`gsh-2015.graph`](http://data.law.di.unimi.it/webdata/gsh-2015/gsh-2015.graph), 8673000442 bytes, SHA-256 not pinned
- **File:** [`gsh-2015.properties`](http://data.law.di.unimi.it/webdata/gsh-2015/gsh-2015.properties), 1387 bytes, SHA-256 not pinned

<!-- END GENERATED -->
