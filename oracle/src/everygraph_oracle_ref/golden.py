# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Golden files: known answers on small public graphs, each value computed by at least
two independent libraries and stored with its provenance in oracle/golden/<name>.json.

Sources: Zachary's karate club is vendored (78 edges, as shipped by NetworkX; checked
against igraph's own copy). The SNAP graphs (all under 1 MB) are not redistributed:
`--fetch` downloads them into the cache and verifies their SHA-256. When the dataset
registry of issue #5 lands, DATASETS moves into its datasets.toml and `--fetch` calls
its fetcher; the golden files keep their `source` block, extended with the registry
entry's name.

Conventions: node ids are the rank of the original id among the ids that appear in the
file (a node that appears in no row does not exist); semantics are Everygraph's defaults
(self-loops dropped and counted, repeated pairs analysed once); triangles and core
numbers on the undirected projection.
"""

from __future__ import annotations

import gzip
import hashlib
import json
import os
import shutil
import urllib.request
from pathlib import Path

import igraph as ig
import networkx as nx
import scipy.sparse.csgraph as csg

from .graph import Graph, project, to_igraph, to_networkx, to_scipy
from .refs import _triangles_scipy
from .run import versions

ORACLE = Path(__file__).resolve().parents[2]
GOLDEN = ORACLE / "golden"
FORMAT = "everygraph-oracle-golden"

SNAP_TERMS = "SNAP datasets, Stanford Network Analysis Project (https://snap.stanford.edu/data/); cite Leskovec and Krevl, SNAP Datasets, 2014."

DATASETS = [
    {
        "name": "karate",
        "file": "karate.tsv",
        "vendored": True,
        "directed": False,
        "origin": "W. W. Zachary, An information flow model for conflict and fission in small groups, J. Anthropological Research 33 (1977); the 78-edge version shipped as networkx.karate_club_graph() and igraph.Graph.Famous('Zachary')",
        "terms": "Published scientific data, redistributed by NetworkX (BSD) and igraph (GPL); 78 edges.",
    },
    {
        "name": "email-Eu-core",
        "file": "email-Eu-core.txt",
        "url": "https://snap.stanford.edu/data/email-Eu-core.txt.gz",
        "sha256": "4b47acdb80197b085fe63c819c357ae488131ee904ed93d1b219a68b0f9e245f",
        "directed": True,
        "page": "https://snap.stanford.edu/data/email-Eu-core.html",
        "published": {"n": 1005, "m": 25571, "largest_wcc": 986, "largest_scc": 803, "triangle_total": 105461},
    },
    {
        "name": "ca-GrQc",
        "file": "ca-GrQc.txt",
        "url": "https://snap.stanford.edu/data/ca-GrQc.txt.gz",
        "sha256": "a254442cdf5d684712578b630c2e0d7543518ab154ef2341cabb607572ce7230",
        "directed": False,
        "page": "https://snap.stanford.edu/data/ca-GrQc.html",
        "published": {"n": 5242, "m": 14496, "largest_wcc": 4158, "triangle_total": 48260},
    },
    {
        "name": "ego-Facebook",
        "file": "facebook_combined.txt",
        "url": "https://snap.stanford.edu/data/facebook_combined.txt.gz",
        "sha256": "125e84db872eeba443d270c70315c256b0af43a502fcfe51f50621166ad035d7",
        "directed": False,
        "page": "https://snap.stanford.edu/data/ego-Facebook.html",
        "published": {"n": 4039, "m": 88234, "largest_wcc": 4039, "triangle_total": 1612010},
    },
    {
        "name": "p2p-Gnutella08",
        "file": "p2p-Gnutella08.txt",
        "url": "https://snap.stanford.edu/data/p2p-Gnutella08.txt.gz",
        "sha256": "cb3019062be21e55b03f19a73e88edf34693bafb68ae581621cc7a037fa604a9",
        "directed": True,
        "page": "https://snap.stanford.edu/data/p2p-Gnutella08.html",
        "published": {"n": 6301, "m": 20777, "largest_wcc": 6299, "largest_scc": 2068, "triangle_total": 2383},
    },
]


def default_cache() -> Path:
    return Path(os.environ.get("EVERYGRAPH_ORACLE_CACHE", ORACLE / ".cache"))


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fetch(cache: Path) -> int:
    cache.mkdir(parents=True, exist_ok=True)
    bad = 0
    for d in DATASETS:
        if d.get("vendored"):
            continue
        gz = cache / Path(d["url"]).name
        if not gz.exists() or sha256(gz) != d["sha256"]:
            print(f"fetching {d['url']}")
            tmp = gz.with_suffix(".part")
            with urllib.request.urlopen(d["url"], timeout=60) as r, tmp.open("wb") as f:
                shutil.copyfileobj(r, f)
            tmp.rename(gz)
        if sha256(gz) != d["sha256"]:
            print(f"{gz}: SHA-256 {sha256(gz)}, expected {d['sha256']}")
            bad += 1
            continue
        with gzip.open(gz, "rb") as src, (cache / d["file"]).open("wb") as dst:
            shutil.copyfileobj(src, dst)
        print(f"{d['name']}: {cache / d['file']} (verified)")
    return 1 if bad else 0


def read_edge_list(path: Path, directed: bool, name: str) -> Graph:
    pairs = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line[0] in "#%":
            continue
        a, b = line.split()[:2]
        pairs.append((int(a), int(b)))
    ids = sorted({x for p in pairs for x in p})
    rank = {x: i for i, x in enumerate(ids)}
    return Graph(
        name=name,
        n=len(ids),
        directed=directed,
        src=[rank[u] for u, _ in pairs],
        dst=[rank[v] for _, v in pairs],
        weight=None,
        time=None,
        partition=None,
        keep_loops=False,
        kernels=[],
    )


def write_karate() -> Path:
    path = GOLDEN / "karate.tsv"
    G = nx.karate_club_graph()
    H = ig.Graph.Famous("Zachary")
    a = {tuple(sorted(e)) for e in G.edges()}
    b = {tuple(sorted(e)) for e in H.get_edgelist()}
    if a != b:
        raise SystemExit("NetworkX and igraph disagree on Zachary's karate club")
    lines = ["# Zachary's karate club (78 edges), 0-based ids; identical in NetworkX and igraph"]
    lines += [f"{u}\t{v}" for u, v in sorted(a)]
    path.write_text("\n".join(lines) + "\n")
    return path


def _agree(name: str, by: dict) -> dict:
    vals = list(by.values())
    if any(v != vals[0] for v in vals[1:]):
        raise SystemExit(f"{name}: libraries disagree: { {k: (v if not isinstance(v, list) else v[:10]) for k, v in by.items()} }")
    return {"value": vals[0], "by": sorted(by)}


def _class_stats(labels) -> tuple[int, int]:
    sizes: dict[int, int] = {}
    for x in labels:
        sizes[x] = sizes.get(x, 0) + 1
    return len(sizes), max(sizes.values(), default=0)


def compute(d: dict, path: Path) -> dict:
    g = read_edge_list(path, d["directed"], d["name"])
    p = project(g)
    n = g.n
    G, H = to_networkx(p), to_igraph(p)
    Gu, Hu = to_networkx(p, undirected=True), to_igraph(p, undirected=True)
    A = to_scipy(p)
    v: dict = {}
    v["n"] = {"value": n, "by": ["edge list"]}
    v["m"] = _agree("m", {"networkx": G.number_of_edges(), "igraph": H.ecount(), "scipy": int(A.nnz if p.directed else A.nnz // 2)})
    v["self_loop_rows"] = {"value": p.self_loop_rows, "by": ["edge list"]}
    v["duplicate_rows"] = {"value": p.duplicate_rows, "by": ["edge list"]}
    wcc_nx = list(nx.weakly_connected_components(G) if p.directed else nx.connected_components(G))
    wcc_ig = H.connected_components(mode="weak").membership
    _, wcc_sc = csg.connected_components(A, directed=p.directed, connection="weak")
    v["wcc_count"] = _agree("wcc_count", {"networkx": len(wcc_nx), "igraph": _class_stats(wcc_ig)[0], "scipy": _class_stats(wcc_sc)[0]})
    v["largest_wcc"] = _agree("largest_wcc", {"networkx": max(map(len, wcc_nx)), "igraph": _class_stats(wcc_ig)[1], "scipy": _class_stats(wcc_sc)[1]})
    if p.directed:
        scc_nx = list(nx.strongly_connected_components(G))
        scc_ig = H.connected_components(mode="strong").membership
        _, scc_sc = csg.connected_components(A, directed=True, connection="strong")
        v["scc_count"] = _agree("scc_count", {"networkx": len(scc_nx), "igraph": _class_stats(scc_ig)[0], "scipy": _class_stats(scc_sc)[0]})
        v["largest_scc"] = _agree("largest_scc", {"networkx": max(map(len, scc_nx)), "igraph": _class_stats(scc_ig)[1], "scipy": _class_stats(scc_sc)[1]})
    t_nx = nx.triangles(Gu)
    t_nx = [t_nx[x] for x in range(n)]
    t_ig = [0] * n
    tri = Hu.list_triangles()
    for t in tri:
        for x in t:
            t_ig[x] += 1
    t_sc = _triangles_scipy(p)
    v["triangle_total"] = _agree("triangle_total", {"networkx": sum(t_nx) // 3, "igraph": len(tri), "scipy": sum(t_sc) // 3})
    v["triangles"] = _agree("triangles", {"networkx": t_nx, "igraph": t_ig, "scipy": t_sc})
    c_nx = nx.core_number(Gu)
    c_nx = [c_nx[x] for x in range(n)]
    c_ig = Hu.coreness(mode="all")
    v["core_numbers"] = _agree("core_numbers", {"networkx": c_nx, "igraph": c_ig})
    v["max_core"] = _agree("max_core", {"networkx": max(c_nx, default=0), "igraph": max(c_ig, default=0)})
    hist = [0] * (max(c_nx, default=-1) + 1)
    for c in c_nx:
        hist[c] += 1
    v["core_histogram"] = {"value": hist, "by": sorted(v["core_numbers"]["by"])}
    notes = []
    for key, pub in d.get("published", {}).items():
        v[key]["published"] = {"value": pub, "source": d["page"]}
        if v[key]["value"] != pub:
            why = ""
            if key == "m" and pub - v[key]["value"] == p.self_loop_rows:
                why = "; the difference is the self-loop rows, which SNAP counts as edges and Everygraph drops"
            notes.append(f"{key}: computed {v[key]['value']}, published {pub} ({d['page']}){why}")
    source = {"file": d["file"], "vendored": bool(d.get("vendored")), "sha256_of_file": sha256(path)}
    for k in ("url", "sha256", "page", "origin", "terms"):
        if k in d:
            source[{"sha256": "sha256_of_download"}.get(k, k)] = d[k]
    if "url" in d:
        source["terms"] = SNAP_TERMS
    return {
        "format": FORMAT,
        "version": 1,
        "name": d["name"],
        "source": source,
        "directed": d["directed"],
        "ids": "rank of the original id among the ids that appear in the file",
        "semantics": "Everygraph defaults: self-loops dropped and counted, repeated pairs analysed once; triangles and core numbers on the undirected projection",
        "libraries": versions(),
        "values": v,
        "notes": notes,
    }


def dumps(doc: dict) -> str:
    """JSON with one line per value, so that per-node arrays do not take a line per node."""
    head = {k: v for k, v in doc.items() if k not in ("values", "notes")}
    lines = json.dumps(head, indent=1).rstrip("}").rstrip().rstrip(",")
    vals = ",\n".join(f"  {json.dumps(k)}: {json.dumps(v, separators=(',', ':'))}" for k, v in doc["values"].items())
    notes = json.dumps(doc["notes"])
    return f'{lines},\n "values": {{\n{vals}\n }},\n "notes": {notes}\n}}\n'


def build(cache: Path, write: bool) -> int:
    status = 0
    if write:
        write_karate()
    for d in DATASETS:
        path = GOLDEN / d["file"] if d.get("vendored") else cache / d["file"]
        if not path.exists():
            print(f"{d['name']}: skipped, {path} missing (oracle-ref golden --fetch)")
            continue
        doc = compute(d, path)
        out = GOLDEN / f"{d['name']}.json"
        text = dumps(doc)
        if write:
            out.write_text(text)
            print(f"{d['name']}: wrote {out.name}; notes: {doc['notes'] or 'none'}")
        else:
            old = json.loads(out.read_text())
            if old["values"] != doc["values"]:
                changed = [k for k in doc["values"] if old["values"].get(k) != doc["values"][k]]
                print(f"{d['name']}: MISMATCH in {changed}")
                status = 1
            else:
                print(f"{d['name']}: {len(doc['values'])} values agree with {out.name}")
    return status
