#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Issue #15 (m1-01): checks what the tracer spec left in its store, after the run.

Not timed and not a benchmark. For one graph it reports, as one JSON object:
* the size on disk of every stage output (the chain's disk budget);
* whether every ingest output is byte-identical to the reference CSR in
  $EVERYGRAPH_DATA/csr/<graph> (cmp);
* SHA-256 of the layout (pos.f32, tnodes.bin) and of the tile file of every variant of
  the same chain, to show that loading the CSR into the heap or mapping it, and the
  memory limit, do not change the output;
* which hierarchy each chain used, with its modularity.

Usage: python3 -I verify.py --store DIR --graph G [--data DIR] [--out FILE]
"""
import argparse
import filecmp
import hashlib
import json
import os


def du(p):
    t = 0
    for root, _, fs in os.walk(p):
        for f in fs:
            t += os.lstat(os.path.join(root, f)).st_blocks * 512
    return t


def sha(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        while b := f.read(1 << 24):
            h.update(b)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--store", required=True)
    ap.add_argument("--graph", required=True)
    ap.add_argument("--data", default=os.environ.get("EVERYGRAPH_DATA", "/home/tux/graphlab/data"))
    ap.add_argument("--out")
    a = ap.parse_args()
    st = os.path.join(a.store, a.graph)
    ref = os.path.join(a.data, "csr", a.graph)
    res = {"graph": a.graph, "store": st, "items": {}}
    for d in sorted(os.listdir(st)):
        p = os.path.join(st, d)
        item = {"bytes": du(p)}
        if d.startswith("csr."):
            item["identical_to_reference"] = all(
                filecmp.cmp(os.path.join(p, f), os.path.join(ref, f), shallow=False)
                for f in ["offsets.u64", "targets.u32", "ids.u64"])
        if d.startswith("hier."):
            j = json.load(open(os.path.join(p, "b2.json")))
            item.update(modularity=j["modularity"], levels=j["level_sizes"], communities=j["communities"])
        if d.startswith("layout."):
            item.update(sha256_pos=sha(os.path.join(p, "pos.f32")), sha256_tnodes=sha(os.path.join(p, "tnodes.bin")),
                        source=json.load(open(os.path.join(p, "source.json"))))
        if d.startswith("tiles."):
            item.update(sha256_pmtiles=sha(os.path.join(p, "map.pmtiles")),
                        source=json.load(open(os.path.join(p, "source.json"))))
        res["items"][d] = item
    # identity across variants of one chain
    groups = {}
    for d, item in res["items"].items():
        for k in ("sha256_pos", "sha256_tnodes", "sha256_pmtiles"):
            if k in item:
                chain = d.split(".")[1]
                groups.setdefault(f"{chain}:{k}", set()).add(item[k])
    res["identical_across_variants"] = {k: len(v) == 1 for k, v in sorted(groups.items())}
    res["total_bytes"] = sum(i["bytes"] for i in res["items"].values())
    s = json.dumps(res, indent=1, sort_keys=True)
    if a.out:
        open(a.out, "w").write(s + "\n")
    print(s)


if __name__ == "__main__":
    main()
