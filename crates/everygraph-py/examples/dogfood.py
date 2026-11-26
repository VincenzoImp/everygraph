# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Dogfood of the Python slice: LiveJournal from its SNAP file to a `.eg`, PageRank and
connected components, as a user would type them. Exploratory: no timing printed here is
a measurement (CLAUDE.md § 3).

Run it under a hard memory limit, with the package installed (`maturin develop`):

    systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0 \\
        python crates/everygraph-py/examples/dogfood.py \\
        --data /home/tux/graphlab/data --out /home/tux/graphlab/scratch/m6-01-python-api
"""

from __future__ import annotations

import argparse
import pathlib
import time

import everygraph as eg


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--data", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--memory", default="6GB")
    args = parser.parse_args()

    source = args.data / "raw" / "com-lj.ungraph.txt.gz"
    graph = args.out / "lj.eg"
    g = eg.read(source, out=graph, memory=args.memory)
    print(g)

    started = time.monotonic()
    pr = g.pagerank()
    print(pr.top(5))
    print(f"iterations {pr.meta['iterations']} · converged {pr.meta['converged']} · "
          f"peak {pr.meta['peak_memory_bytes']:,} bytes · {time.monotonic() - started:.1f} s (exploratory)")

    started = time.monotonic()
    wcc = g.components()
    print(wcc.head())
    print(wcc.sizes().head(3))
    print(f"peak {wcc.meta['peak_memory_bytes']:,} bytes · {time.monotonic() - started:.1f} s (exploratory)")

    again = eg.open(graph, memory=args.memory)
    assert (again.num_nodes, again.num_edges) == (g.num_nodes, g.num_edges)
    print(f"reopened {graph}: {again.num_nodes:,} nodes · {again.num_edges:,} edges")


if __name__ == "__main__":
    main()
