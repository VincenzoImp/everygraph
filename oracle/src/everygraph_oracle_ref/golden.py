# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Golden files: known answers on real graphs, each value computed by independent
implementations and stored compactly with its provenance in oracle/golden/<name>.json.

Sources:
- Zachary's karate club is vendored (78 edges, as shipped by NetworkX, checked against
  igraph's own copy).
- The tier-1 datasets of the registry (datasets/datasets.toml, issue #5) are read from a
  data root filled by `everygraph-bench fetch tier1 --data ROOT`.
- Three small SNAP graphs that the registry does not list are downloaded by `--fetch`
  into the oracle cache and checked against their SHA-256.

Each golden file holds a `parse` block: the few rules (separator, comment prefix, id
columns, row filter, integer or string ids) that turn the file into rows. The Rust check
(crates/everygraph-oracle/src/golden.rs) applies the same rules, so both sides read the
same graph from the same bytes.

Conventions: node ids are the rank of the original id among the ids that appear in the
file (numeric order for integers, code-point order for strings); semantics are
Everygraph's defaults (self-loops dropped and counted, repeated pairs analysed once);
triangles, core numbers and degrees on the undirected projection.

Values are compact: counts, histograms, a few exact per-node values, and the SHA-256 of
per-node arrays (core numbers, triangles), each encoded as little-endian u64 in node order.

Witnesses depend on size, so that each run fits its memory limit:
- NetworkX joins igraph and scipy when the graph has at most NX_MAX_PAIRS pairs.
- Above that, igraph and scipy only. Core numbers then have igraph as their one Python
  witness, and the in-process Rust reference (`everygraph-oracle golden`) is the second.
"""

from __future__ import annotations

import gzip
import hashlib
import json
import os
import resource
import shutil
import tomllib
import urllib.request
from pathlib import Path

import igraph as ig
import networkx as nx
import numpy as np
import scipy.sparse as sp
import scipy.sparse.csgraph as csg

from .run import versions

ORACLE = Path(__file__).resolve().parents[2]
REPO = ORACLE.parent
GOLDEN = ORACLE / "golden"
REGISTRY = REPO / "datasets" / "datasets.toml"
FORMAT = "everygraph-oracle-golden"
VERSION = 2
NX_MAX_PAIRS = 600_000

SNAP_TERMS = "SNAP datasets, Stanford Network Analysis Project (https://snap.stanford.edu/data/); cite Leskovec and Krevl, SNAP Datasets, 2014."

WHITESPACE = {"separator": "whitespace", "comment": "#", "src": 0, "dst": 1, "keep": None, "ids": "integer"}

# Graphs outside the registry. Published values from the SNAP pages (2026-11-25).
EXTRA = [
    {
        "name": "karate",
        "source": {"kind": "vendored", "file": "karate.tsv"},
        "directed": False,
        "origin": "W. W. Zachary, An information flow model for conflict and fission in small groups, J. Anthropological Research 33 (1977); the 78-edge version shipped as networkx.karate_club_graph() and igraph.Graph.Famous('Zachary')",
        "published": {"triangle_total": 45},
        "published_source": "the triangle count of the 78-edge version, as listed in NetworkX's and igraph's documentation of the graph",
    },
    {
        "name": "email-Eu-core",
        "source": {"kind": "download", "file": "email-Eu-core.txt.gz", "url": "https://snap.stanford.edu/data/email-Eu-core.txt.gz",
                   "sha256_of_download": "4b47acdb80197b085fe63c819c357ae488131ee904ed93d1b219a68b0f9e245f"},
        "directed": True,
        "page": "https://snap.stanford.edu/data/email-Eu-core.html",
        "published": {"n": 1005, "m": 25571, "largest_wcc": 986, "largest_scc": 803, "triangle_total": 105461},
    },
    {
        "name": "ca-GrQc",
        "source": {"kind": "download", "file": "ca-GrQc.txt.gz", "url": "https://snap.stanford.edu/data/ca-GrQc.txt.gz",
                   "sha256_of_download": "a254442cdf5d684712578b630c2e0d7543518ab154ef2341cabb607572ce7230"},
        "directed": False,
        "page": "https://snap.stanford.edu/data/ca-GrQc.html",
        "published": {"n": 5242, "m": 14496, "largest_wcc": 4158, "triangle_total": 48260},
    },
    {
        "name": "p2p-Gnutella08",
        "source": {"kind": "download", "file": "p2p-Gnutella08.txt.gz", "url": "https://snap.stanford.edu/data/p2p-Gnutella08.txt.gz",
                   "sha256_of_download": "cb3019062be21e55b03f19a73e88edf34693bafb68ae581621cc7a037fa604a9"},
        "directed": True,
        "page": "https://snap.stanford.edu/data/p2p-Gnutella08.html",
        "published": {"n": 6301, "m": 20777, "largest_wcc": 6299, "largest_scc": 2068, "triangle_total": 2383},
    },
]

# Parse rules for the registry's formats, by dataset name; the default is WHITESPACE.
REGISTRY_PARSE = {
    "soc-sign-bitcoin-otc": {**WHITESPACE, "separator": ",", "comment": None},
    "wiki-clickstream-ptwiki-2024-01": {"separator": "\t", "comment": None, "src": 0, "dst": 1, "keep": {"column": 2, "equals": "link"}, "ids": "string"},
}

# Published SNAP figures for registry datasets, where SNAP gives them; checked against the
# dataset pages on 2026-11-25.
REGISTRY_PUBLISHED = {
    "ego-facebook": {"triangle_total": 1612010, "largest_wcc": 4039},
    "com-youtube": {"triangle_total": 3056386, "largest_wcc": 1134890},
    "roadnet-ca": {"triangle_total": 120676, "largest_wcc": 1957027},
    "web-google": {"triangle_total": 13391903, "largest_wcc": 855802, "largest_scc": 434818},
    "soc-epinions1": {"triangle_total": 1624481, "largest_wcc": 75877, "largest_scc": 32223},
}


def default_cache() -> Path:
    return Path(os.environ.get("EVERYGRAPH_ORACLE_CACHE", ORACLE / ".cache"))


def default_data_root() -> Path | None:
    p = os.environ.get("EVERYGRAPH_ORACLE_DATA") or os.environ.get("EVERYGRAPH_DATA")
    return Path(p) if p else None


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def sha256_u64(values) -> str:
    return hashlib.sha256(np.asarray(values, dtype="<u8").tobytes()).hexdigest()


def registry_entries() -> list[dict]:
    """Tier-1 datasets of the registry, as golden-file entries."""
    reg = tomllib.loads(REGISTRY.read_text())
    out = []
    for ds in reg["dataset"]:
        if ds.get("tier") != 1:
            continue
        (f,) = ds["file"]
        page = ds.get("page", "")
        out.append({
            "name": ds["name"],
            "source": {"kind": "registry", "dataset": ds["name"], "file": f["name"], "sha256_of_download": f["sha256"]},
            "directed": ds["directed"],
            "parse": REGISTRY_PARSE.get(ds["name"], WHITESPACE),
            "page": page,
            "terms": ds.get("licence", ""),
            "published": dict(REGISTRY_PUBLISHED.get(ds["name"], {})),
            "registry_counts": {"nodes": ds["nodes"], "edges": ds["edges"], "note": ds.get("counts_note", "")},
        })
    return out


def entries() -> list[dict]:
    return [{**e, "parse": e.get("parse", WHITESPACE)} for e in EXTRA] + registry_entries()


def source_path(e: dict, cache: Path, data_root: Path | None) -> Path | None:
    s = e["source"]
    if s["kind"] == "vendored":
        return GOLDEN / s["file"]
    if s["kind"] == "download":
        return cache / s["file"]
    if data_root is None:
        return None
    return data_root / s["dataset"] / s["file"]


def fetch(cache: Path) -> int:
    """Downloads the non-registry sources; registry ones come from `everygraph-bench fetch`."""
    cache.mkdir(parents=True, exist_ok=True)
    bad = 0
    for e in EXTRA:
        s = e["source"]
        if s["kind"] != "download":
            continue
        path = cache / s["file"]
        if not path.exists() or sha256_file(path) != s["sha256_of_download"]:
            print(f"fetching {s['url']}")
            tmp = path.with_suffix(".part")
            with urllib.request.urlopen(s["url"], timeout=60) as r, tmp.open("wb") as f:
                shutil.copyfileobj(r, f)
            tmp.rename(path)
        if sha256_file(path) != s["sha256_of_download"]:
            print(f"{path}: SHA-256 {sha256_file(path)}, expected {s['sha256_of_download']}")
            bad += 1
        else:
            print(f"{e['name']}: {path} (verified)")
    print("registry datasets: run `everygraph-bench fetch tier1 --data ROOT` and pass --data-root ROOT")
    return 1 if bad else 0


def _open_text(path: Path):
    if path.suffix == ".gz":
        return gzip.open(path, "rt", encoding="utf-8", newline="")
    return path.open("r", encoding="utf-8", newline="")


def read_rows(path: Path, parse: dict) -> tuple[int, np.ndarray, np.ndarray]:
    """Rows of the file under `parse`, relabelled densely by rank; (n, src, dst)."""
    sep = None if parse["separator"] == "whitespace" else parse["separator"]
    comment = parse.get("comment")
    keep = parse.get("keep")
    si, di = parse["src"], parse["dst"]
    a, b = [], []
    with _open_text(path) as f:
        for line in f:
            line = line.rstrip("\r\n")
            if not line.strip() or (comment and line.startswith(comment)):
                continue
            cols = line.split(sep)
            if keep is not None and (len(cols) <= keep["column"] or cols[keep["column"]] != keep["equals"]):
                continue
            a.append(cols[si].strip())
            b.append(cols[di].strip())
    if parse["ids"] == "integer":
        src = np.array(a, dtype=np.int64)
        dst = np.array(b, dtype=np.int64)
        ids = np.unique(np.concatenate([src, dst]))
        return len(ids), np.searchsorted(ids, src), np.searchsorted(ids, dst)
    names = sorted(set(a) | set(b))
    rank = {s: i for i, s in enumerate(names)}
    return len(names), np.array([rank[x] for x in a], dtype=np.int64), np.array([rank[x] for x in b], dtype=np.int64)


def _agree(name: str, by: dict) -> dict:
    vals = list(by.values())
    if any(v != vals[0] for v in vals[1:]):
        raise SystemExit(f"{name}: witnesses disagree: { {k: v for k, v in by.items()} }")
    return {"value": vals[0], "by": sorted(by)}


def _labels_stats(labels) -> tuple[int, int]:
    _, counts = np.unique(np.asarray(labels), return_counts=True)
    return int(len(counts)), int(counts.max()) if len(counts) else 0


def compute(e: dict, path: Path) -> dict:
    n, src, dst = read_rows(path, e["parse"])
    directed = e["directed"]
    rows = len(src)
    loop = src == dst
    s, d = src[~loop], dst[~loop]
    lo, hi = np.minimum(s, d), np.maximum(s, d)
    und_keys = np.unique(lo * n + hi) if n else np.zeros(0, dtype=np.int64)
    if directed:
        own = np.unique(s * n + d) if n else np.zeros(0, dtype=np.int64)
    else:
        own = und_keys
    m = len(own)
    du, dv = (own // n, own % n) if n else (own, own)
    uu, uv = (und_keys // n, und_keys % n) if n else (und_keys, und_keys)
    v: dict = {}
    v["n"] = {"value": int(n), "by": ["edge list"]}
    v["rows"] = {"value": int(rows), "by": ["edge list"]}
    v["self_loop_rows"] = {"value": int(loop.sum()), "by": ["edge list"]}
    v["duplicate_rows"] = {"value": int(len(s) - m), "by": ["edge list"]}

    H = ig.Graph(n=int(n), edges=np.column_stack([du, dv]).tolist(), directed=directed)
    Hu = H if not directed else ig.Graph(n=int(n), edges=np.column_stack([uu, uv]).tolist(), directed=False)
    A = sp.csr_array((np.ones(m), (du, dv)), shape=(n, n))
    Au = sp.csr_array((np.ones(2 * len(und_keys)), (np.concatenate([uu, uv]), np.concatenate([uv, uu]))), shape=(n, n))
    small = len(und_keys) <= NX_MAX_PAIRS
    if small:
        G = nx.DiGraph() if directed else nx.Graph()
        G.add_nodes_from(range(n))
        G.add_edges_from(zip(du.tolist(), dv.tolist()))
        Gu = G.to_undirected() if directed else G

    by = {"igraph": H.ecount(), "scipy": int(A.nnz)}
    if small:
        by["networkx"] = G.number_of_edges()
    v["m"] = _agree("m", by)

    deg = np.diff(Au.indptr).astype(np.int64)
    v["max_degree"] = {"value": int(deg.max()) if n else 0, "by": ["scipy"]}

    wcc = {"igraph": _labels_stats(H.connected_components(mode="weak").membership),
           "scipy": _labels_stats(csg.connected_components(A, directed=directed, connection="weak")[1])}
    if small:
        comps = list(nx.weakly_connected_components(G) if directed else nx.connected_components(G))
        wcc["networkx"] = (len(comps), max(map(len, comps), default=0))
    v["wcc_count"] = _agree("wcc_count", {k: x[0] for k, x in wcc.items()})
    v["largest_wcc"] = _agree("largest_wcc", {k: x[1] for k, x in wcc.items()})
    if directed:
        scc = {"igraph": _labels_stats(H.connected_components(mode="strong").membership),
               "scipy": _labels_stats(csg.connected_components(A, directed=True, connection="strong")[1])}
        if small:
            comps = list(nx.strongly_connected_components(G))
            scc["networkx"] = (len(comps), max(map(len, comps), default=0))
        v["scc_count"] = _agree("scc_count", {k: x[0] for k, x in scc.items()})
        v["largest_scc"] = _agree("largest_scc", {k: x[1] for k, x in scc.items()})

    # Triangles per node. scipy: products over the degree-ordered orientation (O(m^1.5)).
    key = deg * (n + 1) + np.arange(n)
    fwd = key[uu] < key[uv]
    a_, b_ = np.where(fwd, uu, uv), np.where(fwd, uv, uu)
    U = sp.csr_array((np.ones(len(a_)), (a_, b_)), shape=(n, n))
    C = (U @ U).multiply(U)
    t_sc = (np.asarray(C.sum(axis=1)).ravel() + np.asarray(C.sum(axis=0)).ravel()
            + np.asarray(U.multiply(U.T @ U).sum(axis=1)).ravel())
    t_sc = np.rint(t_sc).astype(np.int64)
    # igraph: local transitivity times the number of neighbour pairs, rounded.
    cl = np.nan_to_num(np.asarray(Hu.transitivity_local_undirected(mode="zero"), dtype=np.float64))
    t_ig = np.rint(cl * deg * (deg - 1) / 2).astype(np.int64)
    tri = {"scipy": sha256_u64(t_sc), "igraph": sha256_u64(t_ig)}
    tot = {"scipy": int(t_sc.sum() // 3), "igraph": int(t_ig.sum() // 3)}
    if small:
        t_nx = nx.triangles(Gu)
        t_nx = np.array([t_nx[x] for x in range(n)], dtype=np.int64)
        tri["networkx"] = sha256_u64(t_nx)
        tot["networkx"] = int(t_nx.sum() // 3)
    v["triangle_total"] = _agree("triangle_total", tot)
    v["triangles_sha256"] = _agree("triangles_sha256", tri)

    c_ig = np.asarray(Hu.coreness(mode="all"), dtype=np.int64)
    core = {"igraph": sha256_u64(c_ig)}
    if small:
        cn = nx.core_number(Gu)
        core["networkx"] = sha256_u64([cn[x] for x in range(n)])
    v["core_numbers_sha256"] = _agree("core_numbers_sha256", core)
    v["max_core"] = {"value": int(c_ig.max()) if n else 0, "by": sorted(core)}
    v["core_histogram"] = {"value": np.bincount(c_ig).astype(int).tolist() if n else [], "by": sorted(core)}

    # A few exact per-node values: first node, last node, the node of largest degree.
    picks = sorted({0, n - 1, int(np.argmax(deg))}) if n else []
    v["samples"] = {"value": [[p, int(deg[p]), int(c_ig[p]), int(t_sc[p])] for p in picks],
                    "by": ["scipy", "igraph"], "columns": ["node", "degree", "core", "triangles"]}

    notes = []
    for k, pub in e.get("published", {}).items():
        v[k]["published"] = {"value": pub, "source": e.get("page") or e.get("published_source", "")}
        if v[k]["value"] != pub:
            why = ""
            if k == "m" and pub - v[k]["value"] == int(loop.sum()):
                why = "; the difference is the self-loop rows, which SNAP counts as edges and Everygraph drops"
            notes.append(f"{k}: computed {v[k]['value']}, published {pub}{why}")
    rc = e.get("registry_counts")
    if rc:
        if rc["nodes"] != n:
            notes.append(f"registry lists {rc['nodes']} nodes, the file has {n} ids that appear in a row")
        if rc["edges"] not in (m, rows):
            notes.append(f"registry lists {rc['edges']} edges; the file has {rows} rows and {m} distinct {'arcs' if directed else 'pairs'}")
    source = dict(e["source"])
    source["sha256_of_file"] = sha256_file(path)
    for k in ("page", "origin", "terms"):
        if e.get(k):
            source[k] = e[k]
    if source["kind"] == "download":
        source["terms"] = SNAP_TERMS
    return {
        "format": FORMAT,
        "version": VERSION,
        "name": e["name"],
        "source": source,
        "parse": e["parse"],
        "directed": directed,
        "ids": "rank of the original id among the ids that appear in a row (numeric order for integer ids, code-point order for strings)",
        "semantics": "Everygraph defaults: self-loops dropped and counted, repeated pairs analysed once; degrees, triangles and core numbers on the undirected projection",
        "witnesses": "networkx, igraph, scipy" if small else f"igraph, scipy (more than {NX_MAX_PAIRS} pairs: no NetworkX); core numbers have igraph as their only Python witness, the in-process Rust reference is the second",
        "libraries": versions(),
        "values": v,
        "notes": notes,
    }


def write_karate() -> Path:
    path = GOLDEN / "karate.tsv"
    G = nx.karate_club_graph()
    H = ig.Graph.Famous("Zachary")
    a = {tuple(sorted(x)) for x in G.edges()}
    b = {tuple(sorted(x)) for x in H.get_edgelist()}
    if a != b:
        raise SystemExit("NetworkX and igraph disagree on Zachary's karate club")
    lines = ["# Zachary's karate club (78 edges), 0-based ids; identical in NetworkX and igraph"]
    lines += [f"{u}\t{v}" for u, v in sorted(a)]
    path.write_text("\n".join(lines) + "\n")
    return path


def dumps(doc: dict) -> str:
    """JSON with one line per value."""
    head = {k: x for k, x in doc.items() if k not in ("values", "notes")}
    lines = json.dumps(head, indent=1).rstrip("}").rstrip().rstrip(",")
    vals = ",\n".join(f"  {json.dumps(k)}: {json.dumps(x, separators=(',', ':'))}" for k, x in doc["values"].items())
    return f'{lines},\n "values": {{\n{vals}\n }},\n "notes": {json.dumps(doc["notes"])}\n}}\n'


def build(cache: Path, data_root: Path | None, write: bool, memory_gib: float = 6.0, only: list[str] | None = None) -> int:
    limit = int(memory_gib * (1 << 30))
    resource.setrlimit(resource.RLIMIT_AS, (limit, limit))
    status = 0
    if write:
        write_karate()
    for e in entries():
        if only and e["name"] not in only:
            continue
        path = source_path(e, cache, data_root)
        if path is None or not path.exists():
            print(f"{e['name']}: skipped, source {path or '(no data root)'} missing")
            continue
        out = GOLDEN / f"{e['name']}.json"
        try:
            doc = compute(e, path)
        except MemoryError:
            print(f"{e['name']}: SKIPPED, exceeded the memory limit of {memory_gib} GiB")
            status = 1
            continue
        peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024
        if write:
            out.write_text(dumps(doc))
            print(f"{e['name']}: wrote {out.name} ({doc['witnesses'].split(' (')[0]}; peak so far {peak:.0f} MiB); notes: {doc['notes'] or 'none'}")
        else:
            if not out.exists():
                print(f"{e['name']}: no golden file")
                status = 1
                continue
            old = json.loads(out.read_text())
            if old["values"] != json.loads(dumps(doc))["values"]:
                changed = [k for k in doc["values"] if old["values"].get(k) != doc["values"][k]]
                print(f"{e['name']}: MISMATCH in {changed}")
                status = 1
            else:
                print(f"{e['name']}: {len(doc['values'])} values agree with {out.name}")
    return status
