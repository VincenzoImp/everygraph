# SPDX-License-Identifier: Apache-2.0 OR MIT
"""`eg.read`, `eg.open`, the graph's properties and `nodes()`."""

from __future__ import annotations

import gzip

import pyarrow as pa
import pytest
from conftest import write_edges

import everygraph as eg


def test_a_small_file_stays_in_memory_and_counts_match(small):
    g_nx, path = small
    g = eg.read(path, progress=False)
    assert g.file is None
    assert (g.num_nodes, g.num_edges, g.directed) == (g_nx.number_of_nodes(), g_nx.number_of_edges(), False)
    assert sorted(pa.table(g.nodes())["node"].to_pylist()) == sorted(g_nx.nodes())


def test_out_writes_a_file_that_open_reads_back(tmp_path):
    path = write_edges(tmp_path / "e.csv", [(1, 2), (2, 3), (3, 1), (7, 8)])
    g = eg.read(path, out=tmp_path / "e.eg", progress=False)
    assert g.file == tmp_path / "e.eg"
    h = eg.open(tmp_path / "e.eg")
    assert (h.num_nodes, h.num_edges, h.directed) == (5, 4, False)
    assert pa.table(h.nodes())["node"].to_pylist() == pa.table(g.nodes())["node"].to_pylist()
    assert not list(tmp_path.glob(".everygraph-*")), "scratch is removed on success"
    assert not list(tmp_path.glob("*.partial"))


def test_the_same_read_again_opens_the_cache(tmp_path, capsys):
    path = write_edges(tmp_path / "e.csv", [(1, 2), (2, 3)])
    eg.read(path, out=tmp_path / "e.eg", progress=False)
    stamp = (tmp_path / "e.eg").stat().st_mtime_ns
    g = eg.read(path, out=tmp_path / "e.eg")
    assert (tmp_path / "e.eg").stat().st_mtime_ns == stamp
    assert "cached" in capsys.readouterr().err
    assert g.num_edges == 2


def test_another_graph_in_the_way_is_refused_then_replaced(tmp_path):
    path = write_edges(tmp_path / "e.csv", [(1, 2), (2, 3)])
    eg.read(path, out=tmp_path / "e.eg", progress=False)
    with pytest.raises(eg.OutputExists) as e:
        eg.read(path, out=tmp_path / "e.eg", directed=True, progress=False)
    assert isinstance(e.value, FileExistsError)
    assert e.value.holds == "other graph"
    assert "overwrite=True" in str(e.value)
    g = eg.read(path, out=tmp_path / "e.eg", directed=True, overwrite=True, progress=False)
    assert g.directed


def test_a_file_not_ours_is_never_replaced_without_overwrite(tmp_path):
    path = write_edges(tmp_path / "e.csv", [(1, 2)])
    (tmp_path / "e.eg").write_text("mine")
    with pytest.raises(eg.OutputExists) as e:
        eg.read(path, out=tmp_path / "e.eg", progress=False)
    assert e.value.holds == "not ours"
    assert (tmp_path / "e.eg").read_text() == "mine"


def test_direction_comes_from_the_column_names_or_the_caller(tmp_path):
    named = write_edges(tmp_path / "t.csv", [(1, 2), (2, 1)], header="from,to")
    assert eg.read(named, progress=False).directed
    assert not eg.read(named, directed=False, progress=False).directed
    assert eg.read(named, progress=False).num_edges == 2
    assert eg.read(named, directed=False, progress=False).num_edges == 2


def test_self_loops_and_duplicates(tmp_path, capsys):
    path = write_edges(tmp_path / "e.csv", [(1, 1), (1, 2), (2, 1), (1, 2), (2, 3)])
    g = eg.read(path)
    assert "dropped: 1 self-loop" in capsys.readouterr().err
    assert (g.num_nodes, g.num_edges) == (3, 4)
    assert eg.read(path, duplicates="collapse", progress=False).num_edges == 2
    assert eg.read(path, self_loops="keep", progress=False).num_edges == 5


def test_text_identifiers_come_back_as_text(tmp_path):
    path = write_edges(tmp_path / "e.csv", [("alice", "bob"), ("bob", "carol")], header=None)
    t = pa.table(eg.read(path, progress=False).nodes())
    assert t.schema.field("node").type == pa.utf8()
    assert sorted(t["node"].to_pylist()) == ["alice", "bob", "carol"]


def test_gzip_input(tmp_path):
    plain = write_edges(tmp_path / "e.csv", [(i, i + 1) for i in range(100)])
    packed = tmp_path / "e.csv.gz"
    packed.write_bytes(gzip.compress(plain.read_bytes()))
    g = eg.read(packed, out=tmp_path / "e.eg", progress=False)
    assert (g.num_nodes, g.num_edges) == (101, 100)
    assert not list(tmp_path.glob(".everygraph-*"))


def test_invalid_rows_are_reported_with_their_lines(tmp_path, validate):
    path = tmp_path / "bad.csv"
    path.write_text("a,b\n1,2\n3\n4,5\n")
    with pytest.raises(eg.InvalidInput) as e:
        eg.read(path, progress=False)
    assert e.value.invalid_rows == 1
    assert e.value.problems[0]["line"] == 3
    assert str(e.value).endswith("Nothing was written.")
    validate(e.value.to_dict(), "urn:everygraph:schema:error:1")


def test_a_damaged_file_is_corrupt(tmp_path):
    path = write_edges(tmp_path / "e.csv", [(1, 2), (2, 3)])
    eg.read(path, out=tmp_path / "e.eg", progress=False)
    data = bytearray((tmp_path / "e.eg").read_bytes())
    data[5000] ^= 0xFF
    (tmp_path / "bad.eg").write_bytes(bytes(data))
    with pytest.raises(eg.CorruptFile):
        eg.open(tmp_path / "bad.eg")
    (tmp_path / "text.eg").write_text("not a graph")
    with pytest.raises(eg.CorruptFile):
        eg.open(tmp_path / "text.eg")


def test_system_errors_stay_python_errors(tmp_path):
    with pytest.raises(FileNotFoundError):
        eg.read(tmp_path / "missing.csv", progress=False)
    with pytest.raises(FileNotFoundError):
        eg.open(tmp_path / "missing.eg")


def test_wrong_arguments_are_type_and_value_errors(tmp_path):
    path = write_edges(tmp_path / "e.csv", [(1, 2)])
    with pytest.raises(ValueError, match="duplicates"):
        eg.read(path, duplicates="many", progress=False)
    with pytest.raises(ValueError, match="not a size"):
        eg.read(path, memory="lots", progress=False)
    with pytest.raises(ValueError, match="more than this machine"):
        eg.read(path, memory="1000TB", progress=False)
    with pytest.raises(ValueError, match="threads"):
        eg.read(path, threads=0, progress=False)
    with pytest.raises(TypeError):
        eg.read(path, progress="yes")
    with pytest.raises(TypeError):
        eg.read(path, colour="blue")
    with pytest.raises(eg.Unsupported):
        eg.read(path, weight="value")


def test_the_budget_refuses_before_any_work(tmp_path):
    path = write_edges(tmp_path / "e.csv", [(i, i + 1) for i in range(1000)])
    with pytest.raises(eg.InsufficientBudget) as e:
        eg.read(path, out=tmp_path / "e.eg", memory="1MB", progress=False)
    assert e.value.budget_bytes == 10**6
    assert e.value.smallest_budget_bytes > 10**6
    assert not (tmp_path / "e.eg").exists()
    way = e.value.ways_forward[0].python
    assert way.startswith("eg.read(") and "memory=" in way
    memory = way.split('memory="')[1].split('"')[0]
    assert eg.read(path, memory=memory, threads=e.value.to_dict().get("threads", None), progress=False).num_edges == 1000


def test_progress_events_follow_the_event_schema(tmp_path, validate):
    path = write_edges(tmp_path / "e.csv", [(i, (i * 7) % 50) for i in range(1, 200)])
    events = []
    eg.read(path, out=tmp_path / "e.eg", progress=events.append)
    assert events, "the build reports its phases"
    for event in events:
        validate(event, "urn:everygraph:schema:event:1")
    finals = [e["name"] for e in events if e["state"] == "done"]
    assert finals == ["reading", "identifiers", "building", "checking"]
    assert len({e["job"] for e in events}) == 1


def test_ctrl_c_is_a_plain_keyboard_interrupt_and_leaves_no_file(tmp_path):
    import _thread
    import threading

    path = write_edges(tmp_path / "big.csv", [(i, (i * 7919) % 200_000) for i in range(400_000)])
    started = threading.Event()

    def interrupt():
        started.wait()
        _thread.interrupt_main()

    threading.Thread(target=interrupt, daemon=True).start()
    with pytest.raises(KeyboardInterrupt):
        eg.read(path, out=tmp_path / "big.eg", progress=lambda e: started.set())
    assert not (tmp_path / "big.eg").exists()
    assert not list(tmp_path.glob("*.partial")) and not list(tmp_path.glob(".everygraph-*"))


def test_an_exception_of_the_progress_function_stops_the_call(tmp_path):
    path = write_edges(tmp_path / "e.csv", [(1, 2), (2, 3)])

    def fail(event):
        raise RuntimeError("stop here")

    with pytest.raises(RuntimeError, match="stop here"):
        eg.read(path, out=tmp_path / "e.eg", progress=fail)
    assert not (tmp_path / "e.eg").exists()
