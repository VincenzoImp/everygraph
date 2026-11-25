# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Tests of the Python side of the oracle: projection, comparators, runners."""

from everygraph_oracle_ref.compare import canonical_labels, close, compare, tolerance
from everygraph_oracle_ref.graph import Graph, project
from everygraph_oracle_ref.refs import compute


def graph(edges, n, directed=False, keep_loops=False, weight=None):
    return Graph(
        name="t",
        n=n,
        directed=directed,
        src=[u for u, _ in edges],
        dst=[v for _, v in edges],
        weight=weight,
        time=None,
        partition=None,
        keep_loops=keep_loops,
        kernels=[],
    )


def test_projection_counts_add_up():
    g = graph([(0, 1), (1, 0), (1, 1), (1, 2), (2, 1), (2, 1)], 4)
    p = project(g)
    assert p.self_loop_rows == 1
    assert p.duplicate_rows == 3
    assert len(p.arcs) == 2
    assert p.self_loop_rows + p.duplicate_rows + len(p.arcs) == len(g.src)


def test_weights_aggregate_in_row_order():
    g = graph([(0, 1), (1, 0), (0, 1)], 2, weight=[0.5, 2.0, 0.25])
    a = project(g).arcs[(0, 1)]
    assert (a.mult, a.wsum, a.wmin) == (3, 2.75, 0.25)


def test_labels_compare_by_class():
    assert canonical_labels([7, 7, 3, 7, 3]) == [0, 0, 2, 0, 2]
    assert compare("wcc", 3, {"labels": [5, 5, 9]}, {"labels": [0, 0, 2]}) is None
    assert compare("wcc", 3, {"labels": [5, 9, 9]}, {"labels": [0, 0, 2]}) is not None


def test_tolerance_is_shared_with_rust():
    t = tolerance("pagerank")
    assert t["rel"] == 1e-9 and t["abs_over_n"] == 1e-9
    assert close(0.5, 0.5 + 1e-12, 2, t)
    assert not close(0.5, 0.5 + 1e-6, 2, t)


def test_libraries_agree_on_a_small_graph():
    g = graph([(0, 1), (1, 2), (2, 0), (2, 3)], 5)
    p = project(g)
    for kernel in [{"kernel": "triangles"}, {"kernel": "core_number"}, {"kernel": "wcc"}, {"kernel": "bfs", "source": 0}]:
        values = compute(p, None, kernel)
        first = next(iter(values.values()))
        assert len(values) >= 2
        for v in values.values():
            assert compare(kernel["kernel"], g.n, v, first) is None, (kernel, values)


def test_kept_loop_adds_two_to_the_degree_once():
    g = graph([(0, 0), (0, 0), (0, 1)], 2, keep_loops=True)
    values = compute(project(g), None, {"kernel": "degree"})
    assert {k: v["ints"] for k, v in values.items()} == {"networkx": [3, 1], "igraph": [3, 1], "scipy": [3, 1]}
