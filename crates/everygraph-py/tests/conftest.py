# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Fixtures: edge lists written to a temporary directory, small random graphs, and the
JSON schemas of `docs/ux-contract.md`."""

from __future__ import annotations

import json
import pathlib
import random
import re

import networkx as nx
import pytest

ROOT = pathlib.Path(__file__).resolve().parents[3]


def write_edges(path: pathlib.Path, edges, header: str | None = "a,b", sep: str = ",") -> pathlib.Path:
    lines = [] if header is None else [header]
    lines += [f"{u}{sep}{v}" for u, v in edges]
    path.write_text("\n".join(lines) + "\n")
    return path


def random_graphs():
    """Small undirected graphs of several shapes, seeded."""
    rng = random.Random(7)
    graphs = [
        ("path", nx.path_graph(12)),
        ("star", nx.star_graph(9)),
        ("complete", nx.complete_graph(7)),
        ("barbell", nx.barbell_graph(5, 3)),
        ("grid", nx.grid_2d_graph(5, 6)),
        ("two-parts", nx.disjoint_union(nx.cycle_graph(5), nx.path_graph(4))),
    ]
    for i in range(6):
        graphs.append((f"gnm-{i}", nx.gnm_random_graph(rng.randint(20, 80), rng.randint(30, 300), seed=i)))
    out = []
    for name, g in graphs:
        g.remove_edges_from(list(nx.selfloop_edges(g)))
        g.remove_nodes_from(list(nx.isolates(g)))
        g = nx.convert_node_labels_to_integers(g)
        if g.number_of_edges():
            out.append((name, g))
    return out


@pytest.fixture(params=random_graphs(), ids=lambda p: p[0])
def small(request, tmp_path):
    """A NetworkX graph and the path of its edge list."""
    name, g = request.param
    rng = random.Random(name)
    edges = list(g.edges())
    rng.shuffle(edges)
    return g, write_edges(tmp_path / f"{name}.csv", edges)


@pytest.fixture(scope="session")
def schemas():
    """The schemas of `docs/ux-contract.md`, by `$id`, as a `referencing` registry."""
    from referencing import Registry, Resource

    text = (ROOT / "docs" / "ux-contract.md").read_text()
    found = {}
    for block in re.findall(r"```json\n(.*?)```", text, flags=re.S):
        try:
            doc = json.loads(block)
        except json.JSONDecodeError:
            continue
        if isinstance(doc, dict) and str(doc.get("$id", "")).startswith("urn:everygraph:schema:"):
            found[doc["$id"]] = doc
    registry = Registry().with_resources((k, Resource.from_contents(v)) for k, v in found.items())
    return found, registry


@pytest.fixture
def validate(schemas):
    import jsonschema

    found, registry = schemas

    def check(document, schema_id):
        jsonschema.Draft202012Validator(found[schema_id], registry=registry).validate(document)

    return check
