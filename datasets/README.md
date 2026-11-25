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
$B fetch tier1               # the CI set: about 57 MB, plus the generated graphs
$B fetch com-livejournal     # one dataset by name
$B fetch tier2               # tiers 1 and 2: about 14 GB, mostly Friendster and Twitter
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
  failed check deletes the download.
- **Local copies.** `--from-dir DIR` takes a file of the same name from `DIR` instead of
  downloading it, after the same checks, as a hard link when possible (the directory is
  only read; do not then modify the linked file).
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
- **MovieLens 100K** (`ml-100k`) has the strictest terms: research use only, no
  redistribution, **no commercial or revenue-bearing use** without permission from
  GroupLens. It is in tier 1 as the bipartite graph because no permissively licensed
  alternative of that size was found; replacing it is worth doing before any commercial
  use of the test suite.

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
| [`soc-sign-bitcoin-otc`](https://snap.stanford.edu/data/soc-sign-bitcoin-otc.html) | 1 | weighted | yes | yes | yes | 5,881 | 35,592 | 397.2 kB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/soc-sign-bitcoin-otc.html) † |
| [`college-msg`](https://snap.stanford.edu/data/CollegeMsg.html) | 1 | temporal | yes | no | yes | 1,899 | 59,835 | 345.3 kB | SHA-256 | [No licence stated by SNAP](https://snap.stanford.edu/data/CollegeMsg.html) † |
| [`ml-100k`](https://grouplens.org/datasets/movielens/100k/) | 1 | bipartite | no | yes | yes | 2,625 | 100,000 | 4.9 MB | SHA-256 | [Research use only, no commercial use without permission (GroupLens)](https://files.grouplens.org/datasets/movielens/ml-100k-README.txt) |
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
- **Notes:** Weights are signed: kernels that need positive weights (Leiden, Dijkstra) must map them first, for example to rating + 11 or to |rating|. The registry has no positively weighted co-occurrence graph yet.
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

### `ml-100k`

MovieLens 100K: user-movie ratings, a bipartite graph (GroupLens ml-100k)

- **Format:** ZIP archive; the graph is ml-100k/u.data: 'user\titem\trating\ttimestamp' per line
- **To an edge list:** unzip ml-100k/u.data. User ids 1..943 and item ids 1..1682 overlap: offset items by 943 (or keep two id spaces). Rating 1..5 is the weight, timestamp in seconds since the epoch.
- **Counts:** 943 users + 1,682 movies = 2,625 nodes; 100,000 ratings, as u.info and the README state. Checked on u.data: 100,000 lines, 943 distinct users, 1,682 distinct items, ratings 1..5.
- **Licence / terms:** Research use only, no commercial use without permission (GroupLens). The terms in the archive's README: research use only; no redistribution without permission; no commercial or revenue-bearing use without permission from GroupLens; no implied endorsement; acknowledge use in publications. Check that a use fits before relying on it.
- **Cite:** F. M. Harper and J. A. Konstan. The MovieLens Datasets: History and Context. ACM TiiS 5(4), 2015.
- **Notes:** The most restrictive terms in the registry; see datasets/README.md. Chosen because no permissively licensed user-item graph of this size was found in the time available.
- **File:** [`ml-100k.zip`](https://files.grouplens.org/datasets/movielens/ml-100k.zip), 4924029 bytes, SHA-256 `50d2a982c66986937beb9ffb3aa76efe955bf3d5c6b761f4e3a7cd717c6a3229`

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
