# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The steps of the user stories of `docs/api-sketch.md` § 1 that this slice covers, as
written there, on small stand-ins for their data sets. Steps that need what is not built
yet (communities, Parquet, views, point queries, `from_edges`) are left out."""

from __future__ import annotations

import random

import pyarrow as pa
import pytest
from conftest import write_edges

import everygraph as eg


@pytest.fixture
def friends(tmp_path):
    """`friends.csv`: a header `user,friend`, one friendship per line, a few self-loops."""
    rng = random.Random(1)
    edges = [(rng.randrange(300), rng.randrange(300)) for _ in range(2000)]
    return write_edges(tmp_path / "friends.csv", edges, header="user,friend")


def test_story_1_read_says_what_it_made(friends, capsys):
    g = eg.read(friends)  # columns found: user, friend
    err = capsys.readouterr().err
    loops = sum(1 for line in friends.read_text().splitlines()[1:] if len(set(line.split(","))) == 1)
    assert f"✓ friends.csv · in memory · {g.num_nodes:,} nodes · {g.num_edges:,} edges" in err
    assert f"dropped: {loops:,} self-loop" in err
    assert not g.directed  # the column names give no direction


def test_story_2_pagerank_top_and_reopen(friends, tmp_path):
    g = eg.read(friends, out=tmp_path / "friends.eg", progress=False)
    pr = g.pagerank()
    top = pr.top(5)
    text = repr(top)
    assert text.splitlines()[-1] == f"top 5 of {g.num_nodes:,} rows by pagerank"
    assert "│ node │ pagerank" in text and "│ rank │" in text
    again = eg.open(tmp_path / "friends.eg").pagerank()
    assert pa.table(again).equals(pa.table(pr))


def test_story_3_the_hubs_by_degree(friends):
    g = eg.read(friends, progress=False)
    hubs = g.top("degree", 10)
    assert list(hubs.columns)[-1] == "rank" and len(hubs) == 10


def test_story_6_joined_with_my_table(friends):
    pl = pytest.importorskip("polars")
    duckdb = pytest.importorskip("duckdb")
    g = eg.read(friends, progress=False)
    labels = pl.DataFrame({"address": list(range(0, 300, 3)), "label": [f"l{i}" for i in range(100)]})
    pr = g.pagerank()
    top = pl.DataFrame(pr.top(50)).join(labels, left_on="node", right_on="address", how="left")
    assert top.height == 50 and top["label"].null_count() < 50
    ranks = duckdb.sql("select count(*) as n, sum(pagerank) as total from pr").fetchall()[0]
    assert ranks[0] == g.num_nodes and abs(ranks[1] - 1) < 1e-9


def test_story_8_an_operation_that_does_not_fit(friends):
    g = eg.read(friends, progress=False)
    with pytest.raises(eg.InsufficientBudget) as refused:
        g.pagerank(memory="1kB")
    text = str(refused.value)
    assert text.startswith("pagerank() needs ")
    assert "Ways forward:" in text and text.endswith("Nothing was changed.")
    # Each refusal names the budget that holds its step; following the ways forward
    # reaches one that holds the whole call (the planner, #45, will name it at once).
    budgets = [refused.value.ways_forward[0].needs_bytes]
    while True:
        try:
            pr = g.pagerank(memory=budgets[-1], progress=False)
            break
        except eg.InsufficientBudget as again:
            budgets.append(again.ways_forward[0].needs_bytes)
            assert budgets[-1] > budgets[-2] and len(budgets) < 20
    assert pr.num_rows == g.num_nodes


def test_story_10_the_command_line_equivalent_of_a_tiny_graph(tmp_path):
    tiny = tmp_path / "tiny.csv"
    tiny.write_text("a,b\n0,1\n0,2\n1,2\n2,3\n2,4\n3,4\n")
    g = eg.read(tiny, progress=False)
    assert repr(g) == "Graph in memory · 5 nodes · 6 edges\n  edges     undirected"
    assert repr(g.nodes()).splitlines()[-1] == "5 rows · in graph order"
