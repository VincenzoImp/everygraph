# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The whole-graph algorithms against NetworkX on small graphs, and what their tables
promise: one row per node in graph order, types, ties, zero copy."""

from __future__ import annotations

import random

import networkx as nx
import pyarrow as pa
import pytest
from conftest import write_edges

import everygraph as eg


def by_node(table, column):
    t = pa.table(table)
    return dict(zip(t["node"].to_pylist(), t[column].to_pylist(), strict=True))


def test_pagerank_matches_networkx(small):
    g_nx, path = small
    g = eg.read(path, progress=False)
    pr = g.pagerank(tol=1e-13, max_iter=10_000, progress=False)
    want = nx.pagerank(g_nx, tol=1e-15, max_iter=10_000)
    got = by_node(pr, "pagerank")
    assert got.keys() == want.keys()
    assert max(abs(got[v] - want[v]) for v in want) < 1e-9
    assert abs(sum(got.values()) - 1) < 1e-12
    assert pr.meta["converged"] and pr.meta["iterations"] > 0


def test_pagerank_is_the_same_at_any_thread_count(small):
    _, path = small
    g = eg.read(path, progress=False)
    one = pa.table(g.pagerank(threads=1, progress=False))
    many = pa.table(g.pagerank(threads=8, progress=False))
    assert one.equals(many)


def test_pagerank_weighs_repeated_pairs_by_their_edges(tmp_path):
    edges = [(0, 1), (0, 1), (0, 1), (1, 2), (2, 3), (3, 0), (2, 0)]
    path = write_edges(tmp_path / "multi.csv", edges)
    g = eg.read(path, progress=False)
    multi = nx.MultiGraph(edges)
    weighted = by_node(g.pagerank(tol=1e-14, max_iter=10_000, progress=False), "pagerank")
    want = nx.pagerank(multi, tol=1e-15, max_iter=10_000)
    assert max(abs(weighted[v] - want[v]) for v in want) < 1e-9
    pairs = by_node(g.pagerank(weight=False, tol=1e-14, max_iter=10_000, progress=False), "pagerank")
    want = nx.pagerank(nx.Graph(edges), tol=1e-15, max_iter=10_000)
    assert max(abs(pairs[v] - want[v]) for v in want) < 1e-9


def test_pagerank_low_precision_is_float32_and_close(small):
    _, path = small
    g = eg.read(path, progress=False)
    low = pa.table(g.pagerank(precision="low", progress=False))
    assert low.schema.field("pagerank").type == pa.float32()
    full = by_node(g.pagerank(progress=False), "pagerank")
    for v, x in zip(low["node"].to_pylist(), low["pagerank"].to_pylist(), strict=True):
        assert abs(x - full[v]) < 1e-5


def test_personalized_pagerank_matches_networkx(small):
    g_nx, path = small
    seeds = sorted(g_nx.nodes())[:2]
    g = eg.read(path, progress=False)
    got = by_node(g.pagerank(personalize=seeds, tol=1e-13, max_iter=10_000, progress=False), "pagerank")
    want = nx.pagerank(g_nx, personalization={s: 1 for s in seeds}, tol=1e-15, max_iter=10_000)
    assert max(abs(got[v] - want[v]) for v in want) < 1e-9


def test_bfs_distances_and_parents(small):
    g_nx, path = small
    g = eg.read(path, progress=False)
    order = {v: i for i, v in enumerate(pa.table(g.nodes())["node"].to_pylist())}
    source = min(g_nx.nodes())
    t = pa.table(g.bfs(source, progress=False))
    assert t.schema.field("distance").type == pa.uint32()
    assert t.schema.field("parent").type == t.schema.field("node").type
    want = nx.single_source_shortest_path_length(g_nx, source)
    for v, d, p in zip(t["node"].to_pylist(), t["distance"].to_pylist(), t["parent"].to_pylist(), strict=True):
        assert d == want.get(v)
        if d is None or d == 0:
            assert p is None
        else:
            closer = [u for u in g_nx[v] if want.get(u) == d - 1]
            assert p == min(closer, key=order.__getitem__)


def test_bfs_max_hops_and_unknown_source(small):
    g_nx, path = small
    g = eg.read(path, progress=False)
    source = min(g_nx.nodes())
    t = pa.table(g.bfs(source, max_hops=1, progress=False))
    reached = {v for v, d in zip(t["node"].to_pylist(), t["distance"].to_pylist(), strict=True) if d is not None}
    assert reached == {source, *g_nx[source]}
    with pytest.raises(eg.UnknownNode) as e:
        g.bfs(10**9)
    assert isinstance(e.value, LookupError)


def test_numeric_columns_cross_without_a_copy(small):
    _, path = small
    pr = eg.read(path, progress=False).pagerank(progress=False)
    first = pa.table(pr)["pagerank"].chunk(0).buffers()[1].address
    again = pa.table(pr)["pagerank"].chunk(0).buffers()[1].address
    assert first == again, "both exports point at the engine's array"


def test_meta_travels_with_the_arrow_schema(small):
    _, path = small
    pr = eg.read(path, progress=False).pagerank(progress=False)
    import json

    meta = json.loads(pa.table(pr).schema.metadata[b"everygraph"])
    assert meta["operation"] == "pagerank" and meta["deterministic"] is True
    assert set(pr.meta) >= {"operation", "parameters", "graph", "engine", "took_seconds",
                            "peak_memory_bytes", "cached", "truncated", "iterations"}
    assert pr.truncated is False


def test_top_orders_by_value_then_graph_order(tmp_path):
    path = write_edges(tmp_path / "star.csv", [(0, i) for i in range(1, 6)] + [(6, 7)])
    pr = eg.read(path, progress=False).pagerank(progress=False)
    full = pa.table(pr)
    rows = list(enumerate(zip(full["node"].to_pylist(), full["pagerank"].to_pylist(), strict=True)))
    want = [node for _, (node, _) in sorted(rows, key=lambda r: (-r[1][1], r[0]))]
    for n in (1, 4, 6, 8, 100):
        top = pa.table(pr.top(n))
        assert top["node"].to_pylist() == want[:n]
        assert top["rank"].to_pylist() == list(range(1, min(n, 8) + 1))
    assert len(pr.top(0)) == 0


def test_a_kernel_that_does_not_fit_is_refused_with_the_smallest_budget(small, validate):
    _, path = small
    g = eg.read(path, progress=False)
    with pytest.raises(eg.InsufficientBudget) as e:
        g.pagerank(memory=64, progress=False)
    doc = e.value.to_dict()
    validate(doc, "urn:everygraph:schema:error:1")
    assert doc["budget_bytes"] == 64 and doc["smallest_budget_bytes"] is None
    assert e.value.ways_forward[0].python.startswith("g.pagerank(memory=")
    assert e.value.ways_forward[0].needs_bytes > 64
    assert str(e.value).splitlines()[-1] == "Nothing was changed."


def directed_graphs():
    """Small directed graphs with sinks, cycles, reciprocal pairs, repeated edges and a
    loop, seeded."""
    rng = random.Random(11)
    graphs = [("cycle", nx.cycle_graph(9, create_using=nx.MultiDiGraph))]
    for i in range(4):
        g = nx.gnm_random_graph(30, 70, seed=rng.randrange(10**6), directed=True)
        multi = nx.MultiDiGraph(g)
        multi.add_edges_from(list(g.edges())[:5])
        multi.add_edge(3, 3)
        graphs.append((f"gnm{i}", multi))
    return graphs


@pytest.mark.parametrize("name,multi", directed_graphs())
def test_a_directed_graph_matches_networkx_through_its_in_edges(tmp_path, name, multi):
    multi = nx.MultiDiGraph(list(multi.edges()))  # the nodes of the edge list only
    path = write_edges(tmp_path / f"{name}.csv", list(multi.edges()), header="from,to")
    g = eg.read(path, self_loops="keep", progress=False)
    assert g.directed
    simple = nx.DiGraph(multi)
    got = by_node(g.pagerank(tol=1e-13, max_iter=10_000, progress=False), "pagerank")
    want = nx.pagerank(multi, tol=1e-15, max_iter=10_000)
    assert max(abs(got[v] - want[v]) for v in want) < 1e-9
    pairs = by_node(g.pagerank(weight=False, tol=1e-13, max_iter=10_000, progress=False), "pagerank")
    want = nx.pagerank(simple, tol=1e-15, max_iter=10_000)
    assert max(abs(pairs[v] - want[v]) for v in want) < 1e-9
    source = min(simple.nodes())
    for direction, view in [("out", simple), ("in", simple.reverse()), ("both", simple.to_undirected())]:
        want = nx.single_source_shortest_path_length(view, source)
        got = by_node(g.bfs(source, direction=direction, progress=False), "distance")
        assert got == {v: want.get(v) for v in simple.nodes()}, direction
    projection = nx.Graph(simple.to_undirected())
    projection.remove_edges_from(nx.selfloop_edges(projection))
    assert by_node(g.kcore(progress=False), "core") == nx.core_number(projection)
    t = pa.table(g.degrees(progress=False))
    assert t.column_names == ["node", "in_degree", "out_degree", "degree"]
    for column, want in [
        ("in_degree", dict(simple.in_degree())),
        ("out_degree", dict(simple.out_degree())),
        ("degree", dict(simple.to_undirected().degree())),
    ]:
        assert by_node(t, column) == want, column
    assert g.top("in_degree", 1).num_rows == 1


def test_an_empty_graph_gives_typed_empty_tables(tmp_path):
    path = tmp_path / "empty.csv"
    path.write_text("from,to\n")
    g = eg.read(path, directed=False, progress=False)
    assert (g.num_nodes, g.num_edges) == (0, 0)
    for table, column, kind in [
        (g.pagerank(progress=False), "pagerank", pa.float64()),
        (g.components(progress=False), "component", pa.uint32()),
        (g.kcore(progress=False), "core", pa.uint32()),
        (g.degrees(progress=False), "degree", pa.uint32()),
        (g.nodes(), "node", pa.uint64()),
    ]:
        t = pa.table(table)
        assert t.num_rows == 0 and t.schema.field(column).type == kind
        assert len(table.head(3)) == 0
        assert repr(table).splitlines()[-1] == "0 rows · in graph order"
        assert repr(table.head(3)).splitlines()[-1].startswith("0 rows")
        assert column == "node" or len(table.top(3, by=column)) == 0
    assert len(g.components(progress=False).sizes()) == 0


def test_degrees_of_unknown_identifiers_are_null_rows(small):
    g_nx, path = small
    g = eg.read(path, progress=False)
    known = min(g_nx.nodes())
    t = g.degrees([known, 10**9, known], progress=False)
    rows = pa.table(t).to_pylist()
    assert rows[0] == rows[2] == {"node": known, "degree": g_nx.degree(known)}
    assert rows[1] == {"node": None, "degree": None}
    assert t.meta["unknown"] == 1


def test_negative_integers_are_found_as_integers(tmp_path):
    path = write_edges(tmp_path / "n.csv", [(-1, 5), (5, 6)], header=None)
    g = eg.read(path, progress=False)
    assert pa.table(g.bfs(-1, progress=False))["distance"].to_pylist().count(None) == 0
    assert len(g.bfs(5, progress=False)) == 3


def test_top_of_a_bfs_ranks_by_distance(small):
    g_nx, path = small
    t = eg.read(path, progress=False).bfs(min(g_nx.nodes()), progress=False).top(1)
    assert t.columns == ["node", "distance", "parent", "rank"]


def test_kernel_events_follow_the_event_schema(small, validate):
    _, path = small
    events = []
    eg.read(path, progress=False).pagerank(progress=events.append)
    assert [e["state"] for e in events] == ["running", "done"]
    for event in events:
        validate(event, "urn:everygraph:schema:event:1")


def test_components_match_networkx_and_are_numbered_by_size(small):
    g_nx, path = small
    g = eg.read(path, progress=False)
    wcc = g.components(progress=False)
    t = pa.table(wcc)
    assert t.schema.field("component").type == pa.uint32()
    order = {v: i for i, v in enumerate(t["node"].to_pylist())}
    got = by_node(wcc, "component")
    parts = sorted(nx.connected_components(g_nx), key=lambda c: (-len(c), min(order[v] for v in c)))
    for number, part in enumerate(parts):
        assert {got[v] for v in part} == {number}
    sizes = pa.table(wcc.sizes())
    assert sizes.column_names == ["component", "nodes"]
    assert sizes["nodes"].to_pylist() == [len(p) for p in parts]
    assert wcc.meta["components"] == len(parts)


def test_components_of_a_directed_graph_are_weak(tmp_path):
    path = write_edges(tmp_path / "d.csv", [(0, 1), (2, 1), (3, 4)], header="from,to")
    got = by_node(eg.read(path, progress=False).components(progress=False), "component")
    assert got[0] == got[1] == got[2] != got[3] == got[4]


def test_kcore_matches_networkx(small):
    g_nx, path = small
    got = by_node(eg.read(path, progress=False).kcore(progress=False), "core")
    assert got == nx.core_number(g_nx)


def test_degrees_count_distinct_neighbors(tmp_path, small):
    g_nx, path = small
    g = eg.read(path, progress=False)
    assert by_node(g.degrees(progress=False), "degree") == dict(g_nx.degree())
    some = sorted(g_nx.nodes())[:3][::-1]
    t = pa.table(g.degrees(some, progress=False))
    assert t["node"].to_pylist() == some
    assert t["degree"].to_pylist() == [g_nx.degree(v) for v in some]
    repeated = write_edges(tmp_path / "r.csv", [(0, 1), (1, 0), (0, 1), (1, 2)])
    degree = by_node(eg.read(repeated, progress=False).degrees(progress=False), "degree")
    assert degree == {0: 1, 1: 2, 2: 1}
