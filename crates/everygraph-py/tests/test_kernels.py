# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The whole-graph algorithms against NetworkX on small graphs, and what their tables
promise: one row per node in graph order, types, ties, zero copy."""

from __future__ import annotations

import pathlib
import random
import re

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
    events = []
    with pytest.raises(eg.InsufficientBudget) as e:
        g.pagerank(memory=64, progress=events.append)
    doc = e.value.to_dict()
    validate(doc, "urn:everygraph:schema:error:1")
    smallest = doc["smallest_budget_bytes"]
    assert doc["budget_bytes"] == 64 and smallest > 64
    assert doc["arithmetic"]["count"] == g.num_nodes
    assert e.value.ways_forward[0].python.startswith('g.pagerank(memory="')
    assert e.value.ways_forward[0].needs_bytes == smallest
    text = str(e.value)
    # All it needs comes first, never less than the budget; then the state, its arithmetic.
    assert doc["needs_bytes"] == smallest > doc["budget_bytes"]
    assert text.startswith(f"pagerank() needs {eg._text.size(smallest)}: ")
    assert f"for per-node state ({g.num_nodes:,} nodes × {doc['arithmetic']['bytes_each']} bytes)" in " ".join(text.split())
    flat = " ".join(text.split())
    assert "; the budget is 64 B. The edges stay on disk either way." in flat
    assert text.splitlines()[-1] == "Nothing was changed."
    # One plan first, the error last (G-16); the plan is the refusal's.
    assert [x["event"] for x in events] == ["plan", "error"]
    assert events[0]["plan"]["refusal"]["smallest_budget_bytes"] == smallest
    assert events[1]["error"] == doc
    # The smallest budget runs; one byte less is refused before any work.
    assert g.pagerank(memory=smallest, progress=False).num_rows == g.num_nodes
    with pytest.raises(eg.InsufficientBudget):
        g.pagerank(memory=smallest - 1, progress=False)


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
    strong = g.components(strong=True, progress=False)
    order = {v: i for i, v in enumerate(pa.table(strong)["node"].to_pylist())}
    parts = sorted(nx.strongly_connected_components(simple), key=lambda c: (-len(c), min(order[v] for v in c)))
    got = by_node(strong, "component")
    assert [{got[v] for v in part} for part in parts] == [{number} for number in range(len(parts))]
    assert strong.meta["components"] == len(parts)


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
    pr = eg.read(path, progress=False).pagerank(progress=events.append)
    kinds = [(e["event"], e.get("state")) for e in events]
    assert kinds == [("plan", None), ("phase", "running"), ("phase", "done"), ("result", None)]
    for event in events:
        validate(event, "urn:everygraph:schema:event:1")
    assert events[0]["plan"] == pr.meta["plan"]
    result = events[-1]
    assert result["counts"] == {"nodes": pr.num_rows, "rows": pr.num_rows}
    assert result["peak_memory_bytes"] == pr.meta["peak_memory_bytes"]
    assert result["peak_memory_bytes"] <= events[0]["plan"]["estimate"]["peak_memory_bytes"]["value"]
    assert len({e["job"] for e in events}) == 1


def test_explain_is_the_plan_of_a_dry_run(small, validate):
    _, path = small
    g = eg.read(path, progress=False)
    events = []
    plan = g.explain("pagerank", memory="1GB")
    assert isinstance(plan, eg.Plan) and plan.fits and plan.refusal is None
    dry = g.pagerank(memory="1GB", dry_run=True, progress=events.append)
    # Free memory is read at each plan (P-2); the rest is the same.
    same = [{k: v for k, v in p.to_dict().items() if k != "machine"} for p in (dry, plan)]
    assert same[0] == same[1]
    assert [e["event"] for e in events] == ["plan", "result"]
    validate(plan.to_dict(), "urn:everygraph:schema:plan:1")
    doc = plan.to_dict()
    assert doc["budget"]["memory_bytes"] == 10**9 and doc["budget"]["memory_from"] == "argument"
    assert doc["estimate"]["peak_memory_bytes"]["value"] == plan.estimate["peak_memory_bytes"]["value"]
    # The step table of `design/preview/api.md` and `notebook.html`, cell 3.
    lines = repr(plan).splitlines()
    assert lines[0] == "Plan for pagerank() on the graph — damping 0.85, max_iter 100, precision full, tol 0.000001"
    assert lines[1].startswith("  fits     peak ") and lines[1].endswith(" of 1.0 GB budget")
    assert lines[2] == "" and lines[3].startswith("  1  keep per-node state") and "memory" in lines[3]
    assert lines[4].startswith("  2  compute ranks") and lines[4].endswith("100 iterations at most")
    assert lines[5] == ""
    assert lines[6].startswith(f"  why      {g.num_nodes:,} nodes × ") and lines[6].endswith(" of state")
    assert lines[-1].startswith("  estimate about ")
    # A plan that does not fit is returned, not raised.
    small_budget = g.explain("pagerank", memory=64)
    assert not small_budget.fits and small_budget.refusal["error"] == "InsufficientBudget"
    assert "Ways forward:" in repr(small_budget)
    with pytest.raises(ValueError, match="operation"):
        g.explain("communities")


def test_a_result_on_a_file_is_kept_and_served_from_it(tmp_path, small, validate):
    _, path = small
    g = eg.read(path, out=tmp_path / "g.eg", progress=False)
    first = g.pagerank(progress=False)
    assert first.meta["cached"] is False
    assert g.explain("pagerank").to_dict()["steps"][0]["step"] == "read the kept result"
    events = []
    again = g.pagerank(progress=events.append)
    assert again.meta["cached"] is True and pa.table(again).equals(pa.table(first))
    assert [(e["event"], e.get("state")) for e in events] == [("plan", None), ("phase", "kept"), ("result", None)]
    for event in events:
        validate(event, "urn:everygraph:schema:event:1")
    # Other parameters are another result; a search is never kept.
    assert g.pagerank(damping=0.5, progress=False).meta["cached"] is False
    assert eg.open(tmp_path / "g.eg").pagerank(progress=False).meta["cached"] is True
    source = pa.table(g.nodes())["node"][0].as_py()
    g.bfs(source, progress=False)
    assert g.bfs(source, progress=False).meta["cached"] is False


def test_threads_and_memory_say_where_they_came_from(small, tmp_path, monkeypatch):
    _, path = small
    g = eg.read(path, out=tmp_path / "g.eg", progress=False)
    budget = g.explain("kcore").to_dict()["budget"]
    assert (budget["memory_from"], budget["threads_from"]) == ("default", "default")
    cores = g.explain("kcore").to_dict()["machine"]["cores"]
    assert cores == eg._native.default_threads() and budget["threads"] <= cores
    asked = g.explain("kcore", threads=3, memory="2GB").to_dict()["budget"]
    assert (asked["memory_from"], asked["threads_from"], asked["memory_bytes"]) == ("argument", "argument", 2 * 10**9)
    assert asked["threads"] <= 3
    opened = eg.open(tmp_path / "g.eg", memory="3GB", threads=2).explain("kcore").to_dict()["budget"]
    assert (opened["memory_from"], opened["threads_from"], opened["memory_bytes"]) == ("graph", "graph", 3 * 10**9)
    monkeypatch.setenv("EVERYGRAPH_MEMORY", "4GB")
    assert g.explain("kcore").to_dict()["budget"]["memory_from"] == "environment"
    events = []
    with pytest.raises(ValueError, match="more than this machine has"):
        g.kcore(memory="1000TB", progress=events.append)
    assert [e["event"] for e in events] == ["error"] and events[0]["error"]["error"] == "UsageError"


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
    with pytest.raises(eg.Unsupported):
        g.components(strong=True, progress=False)


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


@pytest.mark.parametrize("name,multi", directed_graphs()[:3])
def test_the_plan_bounds_what_a_call_reserves(tmp_path, name, multi):
    path = write_edges(tmp_path / f"{name}.csv", list(multi.edges()), header="from,to")
    g = eg.read(path, self_loops="keep", progress=False)
    source = pa.table(g.nodes())["node"][0].as_py()
    calls = [("pagerank", {}), ("pagerank", {"weight": False, "precision": "low"}), ("components", {}),
             ("kcore", {}), ("degrees", {}), ("bfs", {"source": source}),
             ("bfs", {"source": source, "direction": "in"}), ("bfs", {"source": source, "direction": "both"})]
    for operation, parameters in calls:
        plan = g.explain(operation, **parameters)
        result = getattr(g, operation)(**parameters, progress=False)
        assert result.meta["peak_memory_bytes"] <= plan.estimate["peak_memory_bytes"]["value"], operation
    # The projection of a directed graph is merged in a phase of its own, and planned.
    kcore = g.explain("kcore").to_dict()
    assert [p["name"] for p in kcore["phases"]] == ["merging", "kcore"]
    assert "merge both directions into memory" in repr(g.explain("kcore"))
    # Strong components have no bound yet: a refusal names no smallest budget.
    with pytest.raises(eg.InsufficientBudget) as e:
        g.components(strong=True, memory=64, progress=False)
    assert e.value.smallest_budget_bytes is None
    assert e.value.ways_forward[0].python.startswith("g.components(strong=True, memory=")


def test_an_exception_of_the_progress_function_ends_the_events_with_a_stop(small, validate):
    _, path = small
    g = eg.read(path, progress=False)
    events = []

    def third(event):
        events.append(event)
        if len(events) == 3:
            raise RuntimeError("stop here")

    with pytest.raises(RuntimeError, match="stop here"):
        g.pagerank(progress=third)
    assert events[0]["event"] == "plan" and events[-1]["event"] == "stopped"
    assert events[-1]["stopped"]["reason"] == "exception"
    for event in events:
        validate(event, "urn:everygraph:schema:event:1")
    assert g.pagerank(progress=False).num_rows == g.num_nodes, "the graph still answers"


def test_a_long_call_prints_its_plan_block_first(small, monkeypatch, capsys):
    from everygraph import _progress

    _, path = small
    g = eg.read(path, progress=False)
    monkeypatch.setattr(_progress, "T_PLAN", 0.0)
    g.kcore(memory="2GB")
    lines = capsys.readouterr().err.splitlines()
    assert lines[0] == f"the graph · {eg._text.size(g._native.bytes)} · {g.num_nodes:,} nodes · {g.num_edges:,} edges"
    assert [line[:12] for line in lines[2:6]] == ["  machine   ", "  budget    ", "  plan      ", "  estimate  "]
    assert lines[3] == "  budget    2.0 GB memory (memory=)"
    assert lines[4] == "  plan      in memory — the graph fits the memory budget"


def test_a_stop_prints_the_stop_block(capsys):
    from everygraph._progress import Progress

    p = Progress("ingest", ["reading", "identifiers"], "0" * 16, True)
    p("begin", "reading", None, 0.0, 0)
    p("end", "reading", 3, 1.0, 0)
    p("begin", "identifiers", None, 1.0, 0)
    p.stopped("interrupt", "run the same call again", "nothing: there is no e.eg yet")
    block = capsys.readouterr().err.splitlines()[-7:]
    assert block == [
        "Stopped at your request, during identifiers.",
        "",
        "  kept      reading",
        "  dropped   identifiers — that phase starts again from its beginning",
        "  written   nothing: there is no e.eg yet",
        "",
        "  continue:  run the same call again",
    ]


def test_edges_that_do_not_fit_are_streamed_from_the_file(tmp_path):
    n = 200_000
    path = write_edges(tmp_path / "path.csv", [(i, i + 1) for i in range(n - 1)])
    g = eg.read(path, out=tmp_path / "path.eg", progress=False)
    held = g.explain("pagerank", threads=4).to_dict()
    assert held["regime"]["name"] == "in_memory"
    budget = held["estimate"]["peak_memory_bytes"]["value"] + held["regime"]["working_bytes"] - 1
    plan = g.explain("pagerank", threads=4, memory=budget).to_dict()
    assert plan["regime"]["name"] == "streamed" and plan["fits"], plan["regime"]
    io = pathlib.Path("/proc/self/io")
    read = (lambda: int(re.search(r"rchar: (\d+)", io.read_text()).group(1))) if io.exists() else (lambda: 0)
    before = read()
    streamed = g.pagerank(threads=4, memory=budget, progress=False)
    streamed_bytes = read() - before
    assert streamed.meta["plan"]["regime"]["name"] == "streamed"
    # The stream reads the edges by positional reads of the file, every pass; a run that
    # holds them reads them through the map, which no read call counts.
    before = read()
    g.pagerank(threads=4, damping=0.5, progress=False)
    mapped_bytes = read() - before
    if io.exists():
        data = plan["regime"]["working_bytes"]
        assert streamed_bytes >= 2 * data > mapped_bytes, (streamed_bytes, mapped_bytes, data)
    assert streamed.meta["peak_memory_bytes"] <= plan["estimate"]["peak_memory_bytes"]["value"] <= budget
    reference = eg.read(path, progress=False).pagerank(progress=False)
    assert pa.table(streamed).equals(pa.table(reference))


def test_an_error_during_a_run_closes_its_phase(small, validate):
    from everygraph._progress import Progress

    _, path = small
    with pytest.raises(eg.InsufficientBudget) as refused:
        eg.read(path, progress=False).kcore(memory=64, progress=False)
    events = []
    p = Progress("kcore", ["kcore"], "0" * 16, events.append)
    p("begin", "kcore", None, 0.0, 0)
    p.error(refused.value.to_dict())
    assert [(e["event"], e.get("state")) for e in events] == [("phase", "running"), ("phase", "stopped"), ("error", None)]
    for event in events:
        validate(event, "urn:everygraph:schema:event:1")


def test_calls_from_two_threads_share_the_session_in_turn(small):
    import threading

    _, path = small
    g = eg.read(path, progress=False)
    source = pa.table(g.nodes())["node"][0].as_py()
    want = pa.table(g.bfs(source, progress=False))
    got, failed = [], []

    def search():
        try:
            for _ in range(10):
                got.append(pa.table(g.bfs(source, progress=False)))
        except Exception as e:  # noqa: BLE001 - reported below
            failed.append(e)

    workers = [threading.Thread(target=search) for _ in range(4)]
    for w in workers:
        w.start()
    for w in workers:
        w.join(timeout=60)
    assert not failed and len(got) == 40 and all(t.equals(want) for t in got)
