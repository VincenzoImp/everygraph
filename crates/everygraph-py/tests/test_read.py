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


def test_the_budget_gives_large_buffers_pages_of_their_own(tmp_path):
    path = write_edges(tmp_path / "e.csv", [(1, 2)])
    eg.read(path, out=tmp_path / "e.eg", progress=False)
    for g in (eg.read(path, progress=False), eg.open(tmp_path / "e.eg")):
        name, is_pages = g._native.pages
        assert is_pages, name


def test_a_file_that_cannot_be_opened_is_never_replaced(tmp_path):
    path = write_edges(tmp_path / "e.csv", [(1, 2)])
    locked = tmp_path / "locked.eg"
    locked.write_text("the user's file")
    locked.chmod(0)
    try:
        with pytest.raises(eg.OutputExists) as e:
            eg.read(path, out=locked, progress=False)
        assert e.value.holds == "not ours"
        assert str(e.value).endswith("Nothing was changed.")
    finally:
        locked.chmod(0o600)
    assert locked.read_text() == "the user's file"
    (tmp_path / "dir.eg").mkdir()
    with pytest.raises(eg.OutputExists):
        eg.read(path, out=tmp_path / "dir.eg", progress=False)
    assert (tmp_path / "dir.eg").is_dir()


def test_a_changed_input_of_the_same_size_and_time_is_not_the_cache(tmp_path):
    import os

    path = tmp_path / "c.csv"
    path.write_text("1,2\n2,3\n")
    eg.read(path, out=tmp_path / "c.eg", progress=False)
    stat = path.stat()
    path.write_text("1,2\n2,4\n")
    os.utime(path, ns=(stat.st_atime_ns, stat.st_mtime_ns))
    with pytest.raises(eg.OutputExists):
        eg.read(path, out=tmp_path / "c.eg", progress=False)


def test_two_spellings_of_one_path_are_one_cache(tmp_path, capsys):
    (tmp_path / "d").mkdir()
    path = write_edges(tmp_path / "d" / "c.csv", [(1, 2)])
    eg.read(path, out=tmp_path / "d" / "c.eg", progress=False)
    eg.read(tmp_path / "d" / ".." / "d" / "c.csv", out=tmp_path / "d" / "c.eg")
    assert "cached" in capsys.readouterr().err


def test_a_small_compressed_input_stays_in_memory(tmp_path):
    plain = write_edges(tmp_path / "e.csv", [(i, i + 1) for i in range(100)])
    packed = tmp_path / "e.csv.gz"
    packed.write_bytes(gzip.compress(plain.read_bytes()))
    g = eg.read(packed, progress=False)
    assert g.file is None and g.num_edges == 100
    assert not list(tmp_path.glob("*.eg")) and not list(tmp_path.glob(".everygraph-*"))


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
    assert e.value.smallest_budget_bytes is None, "known once the planner exists (#45)"
    assert not (tmp_path / "e.eg").exists()
    way = e.value.ways_forward[0]
    assert way.python.startswith("eg.read(") and "memory=" in way.python
    assert way.needs_bytes > 10**6
    assert eg.read(path, memory=way.needs_bytes, progress=False).num_edges == 1000


def test_a_stopped_call_ends_its_events_with_a_stop(tmp_path, validate):
    path = write_edges(tmp_path / "e.csv", [(1, 2), (2, 3)])
    events = []

    def fail(event):
        events.append(event)
        if event["event"] == "phase" and event["state"] == "done":
            raise RuntimeError("stop here")

    with pytest.raises(RuntimeError):
        eg.read(path, progress=fail)
    assert events[-1]["event"] == "stopped" and events[-1]["stopped"]["reason"] == "exception"
    for event in events:
        validate(event, "urn:everygraph:schema:event:1")


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


def test_two_ctrl_c_end_the_process_at_once(tmp_path):
    import signal
    import subprocess
    import sys
    import time

    path = write_edges(tmp_path / "big.csv", [(i, (i * 7919) % 50_000) for i in range(100_000)])
    script = (
        "import sys, everygraph as eg\n"
        f"g = eg.read({str(path)!r}, progress=False)\n"
        "print('ready', flush=True)\n"
        "g.pagerank(tol=0, max_iter=10**7, progress=False)\n"
        "print('finished', flush=True)\n"
    )
    child = subprocess.Popen([sys.executable, "-c", script], stdout=subprocess.PIPE, text=True)
    assert child.stdout.readline().strip() == "ready"
    time.sleep(0.5)
    child.send_signal(signal.SIGINT)
    time.sleep(0.3)
    child.send_signal(signal.SIGINT)
    started = time.monotonic()
    try:
        assert child.wait(timeout=10) == 130
    finally:
        child.kill()
    assert time.monotonic() - started < 5
    assert "finished" not in child.stdout.read()


def test_an_exception_of_the_progress_function_stops_the_call(tmp_path):
    path = write_edges(tmp_path / "e.csv", [(1, 2), (2, 3)])

    def fail(event):
        raise RuntimeError("stop here")

    with pytest.raises(RuntimeError, match="stop here"):
        eg.read(path, out=tmp_path / "e.eg", progress=fail)
    assert not (tmp_path / "e.eg").exists()
    eg.read(path, out=tmp_path / "e.eg", progress=False)
    before = (tmp_path / "e.eg").read_bytes()
    with pytest.raises(RuntimeError, match="stop here"):
        eg.read(path, out=tmp_path / "e.eg", directed=True, overwrite=True, progress=fail)
    assert (tmp_path / "e.eg").read_bytes() == before, "a stopped call changes nothing"
    assert sorted(p.name for p in tmp_path.iterdir()) == ["e.csv", "e.eg"]
