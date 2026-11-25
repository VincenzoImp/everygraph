#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Issue #15 (m1-01) tracer bullet: glue for the lab binaries, one stage per call.

    text -> CSR (glab ingest) -> communities + hierarchy (b2) -> layout (gmap layout)
         -> tiles (gmap tiles) -> viewer (lab/e-map/viewer, Playwright)

Each call runs one stage of the chain for one graph. `everygraph-bench` runs it inside
its `systemd-run` scope (bench/specs/tracer.toml), so the memory limit and the cgroup
peaks are the harness's; this script adds what the harness cannot know:

* where a stage's input comes from and where its output goes. Outputs are built in the
  case's work directory and, only when the stage succeeds, moved to a persistent store
  (`--store`), where the next stage finds them. A failed stage leaves the store as it
  was, so the spec empties the store at the start of every run (prepare step
  `clear-store`), and every stage prints what it read (`consumed`), which the spec
  records in the row;
* the input files are evicted from the page cache before the tool starts (cold input),
  so that every page the tool reads is charged to its own cgroup. Without it a stage
  would read its predecessor's output from page cache charged to another (dead)
  cgroup, outside the limit being tested;
* the tool's own wall time (`seconds`: process start to exit, loading and writing
  included; eviction and the move into the store excluded);
* disk: the peak of the bytes allocated under the work directory while the tool runs
  (temporary files plus the output being written; sampled every 0.25 s) and the size
  of the output;
* the stage's own figures (modularity, levels, tiles, bytes of the viewer session).

The last line of standard output is one JSON object; the spec reads its metrics from it.
A tool that fails makes this script exit with the tool's status (128 + signal when it
was killed), so the harness records `failed` or `oom`.

Stdlib only; run it with `python3 -I`. No new algorithm lives here: every computation
is one of the lab binaries, called with the options of the lab's own scripts.
"""

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))

# Map-shaped hierarchy, as the one experiment E used (lab/b2-leiden/bench.sh, hier
# variants; FINDINGS § B2 "Hierarchy for the map"): fan-out cap 64 during coarsening,
# levels kept only if they shrink the previous kept level 3x. Plain: no cap, as the
# FINDINGS § B2 Friendster rows (lab/b2-leiden/run_friendster.sh).
HIER_FLAGS = {
    "capped": ["--cap", "64", "--max-passes", "20", "--min-ratio", "3"],
    "plain": ["--min-ratio", "3"],
}
# The Friendster configuration of lab/b2-leiden/run_friendster.sh, used for every graph.
B2_FLAGS = ["--lowmem", "--no-populate"]


def log(*a):
    print("[tracer]", *a, file=sys.stderr, flush=True)


def disk_bytes(path):
    """Bytes allocated (st_blocks) under path, not following symlinks."""
    total = 0
    if os.path.isfile(path) and not os.path.islink(path):
        return os.lstat(path).st_blocks * 512
    for root, dirs, files in os.walk(path):
        for f in files:
            try:
                total += os.lstat(os.path.join(root, f)).st_blocks * 512
            except FileNotFoundError:
                pass
    return total


def evict(paths):
    """Drops the clean pages of every regular file under paths (posix_fadvise
    DONTNEED). The harness runs `sync` before each repetition, so the pages are clean.
    Returns (bytes of files, bytes still resident after eviction, via mincore through
    util-linux fincore)."""
    files = []
    for p in paths:
        if os.path.isdir(p):
            for root, _, fs in os.walk(p):
                files += [os.path.join(root, f) for f in fs]
        elif os.path.isfile(p):
            files.append(p)
    size = 0
    for f in files:
        if os.path.islink(f):
            continue
        fd = os.open(f, os.O_RDONLY)
        try:
            os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
            size += os.fstat(fd).st_size
        finally:
            os.close(fd)
    resident = 0
    if files and shutil.which("fincore"):
        out = subprocess.run(["fincore", "--bytes", "--noheadings", "--raw", "--output", "RES"] + files,
                             capture_output=True, text=True)
        for line in out.stdout.split():
            try:
                resident += int(line)
            except ValueError:
                pass
    return size, resident


class DiskSampler(threading.Thread):
    """Peak of disk_bytes(path) while the tool runs (a thread, not a process: nothing
    is left in the scope)."""

    def __init__(self, path, interval=0.25):
        super().__init__(daemon=True)
        self.path, self.interval, self.peak, self.stop = path, interval, 0, threading.Event()

    def run(self):
        while not self.stop.is_set():
            self.peak = max(self.peak, disk_bytes(self.path))
            self.stop.wait(self.interval)

    def finish(self):
        self.stop.set()
        self.join()
        self.peak = max(self.peak, disk_bytes(self.path))
        return self.peak


def run_tool(argv, work, env=None, stdout_path=None):
    """Runs argv with cwd=work, sampling disk under work. Returns (status, seconds,
    peak disk bytes). status is the exit code, or 128 + signal."""
    log("run:", " ".join(argv))
    sampler = DiskSampler(work)
    sampler.start()
    t0 = time.monotonic()
    out = open(stdout_path, "wb") if stdout_path else subprocess.DEVNULL
    try:
        p = subprocess.run(argv, cwd=work, env=env, stdout=out)
    finally:
        if stdout_path:
            out.close()
    seconds = time.monotonic() - t0
    peak = sampler.finish()
    rc = p.returncode if p.returncode >= 0 else 128 - p.returncode
    if rc != 0:
        log(f"tool exited with status {rc} after {seconds:.1f} s")
    return rc, seconds, peak


def install(src, dst):
    """Moves src into the store as dst, replacing an older dst only now that the new
    one is complete."""
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    old = dst + ".old"
    if os.path.lexists(old):
        rm(old)
    if os.path.lexists(dst):
        os.rename(dst, old)
    os.rename(src, dst)
    if os.path.lexists(old):
        rm(old)


def rm(p):
    if os.path.isdir(p) and not os.path.islink(p):
        shutil.rmtree(p)
    else:
        os.remove(p)


def last_json(path):
    with open(path, "rb") as f:
        lines = [l for l in f.read().decode(errors="replace").splitlines() if l.strip().startswith("{")]
    return json.loads(lines[-1]) if lines else {}


def emit(obj):
    print(json.dumps(obj, sort_keys=True), flush=True)


def missing(a, what):
    """A stage whose input an earlier stage did not produce: fails with a JSON line
    (the spec records `/consumed` from every stage)."""
    emit({"stage": a.stage, "graph": a.graph, "status": "failed", "error": what, "consumed": f"none: {what}"})
    sys.exit(1)


def fail(rc, obj):
    obj["status"] = "failed"
    obj["exit"] = rc
    emit(obj)
    sys.exit(rc)


# --- stages ---------------------------------------------------------------------------

def stage_ingest(a):
    store = os.path.join(a.store, f"csr.{a.tag}")
    out, tmp = os.path.join(a.work, "out"), os.path.join(a.work, "tmp")
    res = {"stage": "ingest", "graph": a.graph, "tag": a.tag, "input": a.input, "budget_gb": a.budget,
           "consumed": a.input}
    size, resident = evict([a.input])
    res.update(input_bytes=size, input_resident_after_evict=resident)
    cmd_tail = ["--out", out, "--tmp", tmp, "--mem-gb", str(a.budget), "--threads", str(a.threads), "--out-mmap"]
    if a.input.endswith(".gz"):
        # From the distributed file: pigz inflates into a pipe (lab/b1-core/bench.sh
        # "pigz-pipe"); both processes are in the scope.
        shell = f"pigz -dc '{a.input}' | '{a.glab}' ingest --input - " + " ".join(f"'{x}'" for x in cmd_tail)
        argv = ["bash", "-o", "pipefail", "-c", shell]
        res["mode_requested"] = "pigz pipe"
    else:
        argv = [a.glab, "ingest", "--input", a.input] + cmd_tail + ["--direct-in"]
        res["mode_requested"] = "plain text, O_DIRECT"
    log_path = os.path.join(a.work, "glab.json")
    rc, secs, peak = run_tool(argv, a.work, stdout_path=log_path)
    res.update(seconds=secs, disk_peak_bytes=peak)
    if rc != 0:
        fail(rc, res)
    j = last_json(log_path)
    res.update(tool_seconds=j.get("seconds"), mode=j.get("mode"), n=j.get("n"), entries=j.get("entries"),
               tool_peak_rss_gb=j.get("peak_rss_gb"), tool_temp_disk_gb=j.get("temp_disk_gb"),
               buckets_on_disk=j.get("buckets_on_disk"))
    shutil.rmtree(tmp, ignore_errors=True)
    res["output_bytes"] = disk_bytes(out)
    install(out, store)
    res.update(status="ok", output=store)
    emit(res)


def stage_communities(a):
    csr = os.path.join(a.store, f"csr.{a.csr}")
    name = f"hier.{a.algo}-{a.variant}.{a.tag}"
    store = os.path.join(a.store, name)
    out = os.path.join(a.work, "hier")
    res = {"stage": "communities", "graph": a.graph, "algo": a.algo, "variant": a.variant, "tag": a.tag,
           "input": csr, "flags": B2_FLAGS + HIER_FLAGS[a.variant] + ["--out-hier"], "consumed": csr}
    size, resident = evict([csr])
    res.update(input_bytes=size, input_resident_after_evict=resident)
    jpath = os.path.join(a.work, "b2.json")
    argv = [a.b2, "run", "--graph", csr, "--name", a.graph, "--algo", a.algo, "--threads", str(a.threads)] \
        + B2_FLAGS + HIER_FLAGS[a.variant] + ["--verbose", "--out-hier", out, "--json", jpath,
                                              "--notes", f"m1-01 tracer: {a.algo} {a.variant}"]
    rc, secs, peak = run_tool(argv, a.work, stdout_path=os.path.join(a.work, "b2.out"))
    res.update(seconds=secs, disk_peak_bytes=peak)
    if rc != 0:
        fail(rc, res)
    j = json.load(open(jpath))
    h = json.load(open(os.path.join(out, "hier.json")))
    res.update(tool_seconds=j["time_s"], modularity=j["modularity"], communities=j["communities"],
               level_sizes=j["level_sizes"], disconnected=j["disconnected"], largest=j["largest"],
               hier_levels=h["levels"], hier_modularity_top=h["modularity_top"],
               fanout_max=[f["max"] for f in h["fanout"]], weight_bits=j["weight_bits"],
               tool_rss_anon_peak_gb=j["rss_anon_peak_kb"] / 2**20, tool_rss_peak_gb=j["rss_peak_kb"] / 2**20,
               hash_levels=j["hash_levels"])
    shutil.copy(jpath, os.path.join(out, "b2.json"))
    res["output_bytes"] = disk_bytes(out)
    install(out, store)
    res.update(status="ok", output=store)
    emit(res)


def hier_candidates(a):
    """The hierarchies a chain may use, best first: those computed under the chain's
    community limit; map-shaped (capped) ones before plain ones, whose modularity is
    higher only because the fan-out cap costs 2-5 points (FINDINGS § B2) while their
    fan-out is unbounded; then by modularity."""
    found = []
    for d in sorted(os.listdir(a.store)):
        if d.startswith("hier.") and d.endswith("." + a.hier_tag) and os.path.isfile(os.path.join(a.store, d, "b2.json")):
            j = json.load(open(os.path.join(a.store, d, "b2.json")))
            found.append((d.split(".")[1].endswith("-capped"), j["modularity"], d))
    found.sort(reverse=True)
    return [[q, d] for _, q, d in found]


def pick_hier(a):
    if a.hier:
        return os.path.join(a.store, a.hier), None
    c = hier_candidates(a)
    if not c:
        missing(a, f"no hierarchy computed under {a.hier_tag} in {a.store}")
    return os.path.join(a.store, c[0][1]), c


def gmap_env(a):
    env = dict(os.environ, GMAP_THREADS=str(a.threads))
    if a.load == "mmap":
        env["GMAP_LOAD"] = "mmap"
    return env


def stage_layout(a):
    csr = os.path.join(a.store, f"csr.{a.csr}")
    hier, cands = pick_hier(a)
    store = os.path.join(a.store, f"layout.{a.chain}.{a.tag}")
    out = os.path.join(a.work, "layout")
    res = {"stage": "layout", "graph": a.graph, "chain": a.chain, "tag": a.tag, "load": a.load,
           "input": csr, "hier": hier, "hier_candidates": cands, "consumed": f"csr={csr} hier={hier}"}
    size, resident = evict([csr, hier])
    res.update(input_bytes=size, input_resident_after_evict=resident)
    argv = [a.gmap, "layout", "--graph", csr, "--hier", hier, "--out", out, "--phi", "0.72"]
    rc, secs, peak = run_tool(argv, a.work, env=gmap_env(a), stdout_path=os.path.join(a.work, "layout.out"))
    res.update(seconds=secs, disk_peak_bytes=peak)
    if rc != 0:
        fail(rc, res)
    j = json.load(open(os.path.join(out, "layout.json")))
    res.update(tool_seconds=j["total_s"], load_seconds=j["load_s"], layout_seconds=j["layout_s"],
               tool_peak_rss_gb=j["peak_rss_mb"] / 1024, tnodes_alive=j["tnodes_alive"], virtual=j["virtual"],
               max_depth=j["max_depth"], area_retention=j["area_retention"])
    res["output_bytes"] = disk_bytes(out)
    with open(os.path.join(out, "source.json"), "w") as f:
        json.dump({"csr": csr, "hier": hier}, f)
    install(out, store)
    res.update(status="ok", output=store)
    emit(res)


def first_existing(a, kind, prefer):
    for t in prefer:
        p = os.path.join(a.store, f"{kind}.{a.chain}.{t}")
        if os.path.exists(p):
            return p
    missing(a, f"no {kind} for chain {a.chain} in {a.store} (looked for {prefer})")


def stage_tiles(a):
    csr = os.path.join(a.store, f"csr.{a.csr}")
    lay = first_existing(a, "layout", [a.tag] + a.fallback)
    store = os.path.join(a.store, f"tiles.{a.chain}.{a.tag}")
    out = os.path.join(a.work, "tiles")
    os.makedirs(out)
    res = {"stage": "tiles", "graph": a.graph, "chain": a.chain, "tag": a.tag, "load": a.load,
           "input": csr, "layout": lay, "consumed": f"csr={csr} layout={lay}"}
    size, resident = evict([csr, lay])
    res.update(input_bytes=size, input_resident_after_evict=resident)
    jpath = os.path.join(out, "tiles.json")
    argv = [a.gmap, "tiles", "--graph", csr, "--layout", lay, "--out", os.path.join(out, "map.pmtiles"), "--json", jpath]
    rc, secs, peak = run_tool(argv, a.work, env=gmap_env(a), stdout_path=os.path.join(a.work, "tiles.out"))
    res.update(seconds=secs, disk_peak_bytes=peak)
    if rc != 0:
        fail(rc, res)
    j = json.load(open(jpath))
    res.update(tool_seconds=j["total_s"], load_seconds=j["load_s"], build_seconds=j["build_s"],
               write_seconds=j["write_s"], tool_peak_rss_gb=j["peak_rss_mb"] / 1024, tiles=j["tiles"],
               zmax=j["zmax"], file_bytes=j["file_bytes"], tile_bytes_gz=j["tile_bytes_gz"],
               search_bytes=j["search_bytes"], tile_gz_median=j["tile_gz_median"], tile_gz_max=j["tile_gz_max"],
               edge_weight_shown_deepest=j["zoom"][-1]["edge_weight_shown"] / max(1, j["zoom"][-1]["edge_weight_total"]))
    res["output_bytes"] = disk_bytes(out)
    with open(os.path.join(out, "source.json"), "w") as f:
        json.dump({"csr": csr, "layout": lay}, f)
    install(out, store)
    res.update(status="ok", output=store)
    emit(res)


def stage_viewer(a):
    tiles = first_existing(a, "tiles", a.prefer)
    src = json.load(open(os.path.join(tiles, "source.json")))
    store = os.path.join(a.store, f"viewer.{a.chain}")
    out = os.path.join(a.work, "viewer")
    os.makedirs(out)
    res = {"stage": "viewer", "graph": a.graph, "chain": a.chain, "tiles": tiles, "layout": src["layout"],
           "consumed": f"tiles={tiles}"}
    # The zoom target: experiment E's own picker (a mid-sized finest community).
    tgt = os.path.join(out, "target.json")
    with open(tgt, "w") as f:
        p = subprocess.run(a.python + [os.path.join(REPO, "lab/e-map/py/pick.py"), src["layout"], src["csr"]], stdout=f)
    if p.returncode != 0:
        fail(p.returncode, res)
    # Server root: the lab viewer and the map, by symlink. serve.mjs binds 127.0.0.1.
    root = os.path.join(a.work, "www")
    os.makedirs(os.path.join(root, "out"))
    os.symlink(os.path.join(REPO, "lab/e-map/viewer"), os.path.join(root, "viewer"))
    os.symlink(os.path.join(tiles, "map.pmtiles"), os.path.join(root, "out", "map.pmtiles"))
    # The capture script imports playwright: it runs from a directory that sees the
    # shared Playwright install (node resolves modules next to the script).
    cap_dir = os.path.join(a.work, "pw")
    os.makedirs(cap_dir)
    os.symlink(os.path.join(a.playwright, "node_modules"), os.path.join(cap_dir, "node_modules"))
    shutil.copy(os.path.join(HERE, "capture.mjs"), cap_dir)
    server = subprocess.Popen(["node", os.path.join(REPO, "lab/e-map/viewer/serve.mjs"), root, str(a.port)],
                              stdout=subprocess.PIPE, stderr=sys.stderr, start_new_session=True)
    res["server_pid"] = server.pid
    try:
        line = server.stdout.readline().decode()
        if "127.0.0.1" not in line:
            fail(1, dict(res, error=f"server did not start: {line!r}"))
        log("server pid", server.pid, line.strip())
        rjson = os.path.join(out, "viewer.json")
        argv = ["node", os.path.join(cap_dir, "capture.mjs"), f"http://127.0.0.1:{a.port}/viewer/index.html?file=../out/map.pmtiles",
                os.path.join(out, f"{a.graph}-{a.chain}"), rjson, tgt]
        rc, secs, _ = run_tool(argv, cap_dir, stdout_path=os.path.join(a.work, "capture.out"))
    finally:
        # Killed by PID (its own session), never by name.
        try:
            os.killpg(server.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        server.wait(timeout=10)
    res["seconds"] = secs
    if rc != 0:
        fail(rc, res)
    v = json.load(open(rjson))
    inter = v["interactions"]
    res.update(renderer=v["renderer"], first_view_bytes=inter[0]["bytes"], first_view_requests=inter[0]["requests"],
               session_bytes=v["total"]["bytes"], session_requests=v["total"]["requests"],
               session_tiles=v["total"]["tiles"], search_bytes=v["search"]["total_bytes_incl_tiles"],
               search_requests=v["search"]["requests"], zooms=v["zooms"], context_lost=v["context_lost"],
               target=json.load(open(tgt)))
    install(out, store)
    res.update(status="ok", output=store)
    emit(res)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("stage", choices=["ingest", "communities", "layout", "tiles", "viewer"])
    ap.add_argument("--store", required=True, help="persistent store of stage outputs (one per graph)")
    ap.add_argument("--work", required=True, help="the case's work directory ({case_work})")
    ap.add_argument("--graph", required=True)
    ap.add_argument("--threads", type=int, default=32)
    ap.add_argument("--tag", default="", help="names this case's output in the store (limit, budget, load)")
    ap.add_argument("--glab"), ap.add_argument("--b2"), ap.add_argument("--gmap")
    ap.add_argument("--input", help="ingest: text edge list, or .gz")
    ap.add_argument("--budget", type=float, default=8, help="ingest: glab --mem-gb")
    ap.add_argument("--csr", help="the ingest tag whose CSR the stage reads")
    ap.add_argument("--algo", choices=["leiden", "louvain"])
    ap.add_argument("--variant", choices=list(HIER_FLAGS), default="capped")
    ap.add_argument("--chain", help="layout/tiles/viewer: name of the chain (a store prefix)")
    ap.add_argument("--hier-tag", help="layout: use the best hierarchy computed under this tag")
    ap.add_argument("--hier", help="layout: use this hierarchy (store name) instead")
    ap.add_argument("--load", choices=["heap", "mmap"], default="heap", help="gmap: how the CSR is loaded")
    ap.add_argument("--fallback", nargs="*", default=[], help="tiles: layout tags to use if the own one is missing")
    ap.add_argument("--prefer", nargs="*", default=[], help="viewer: tiles tags, first existing is shown")
    ap.add_argument("--port", type=int, default=8791)
    ap.add_argument("--playwright", default="/home/tux/graphlab/scratch/pw")
    ap.add_argument("--python", nargs="+", default=["uv", "run", "--no-project", "--offline", "--with", "numpy", "python", "-I"])
    a = ap.parse_args()
    a.store = os.path.join(a.store, a.graph)
    os.makedirs(a.store, exist_ok=True)
    os.makedirs(a.work, exist_ok=True)
    globals()["stage_" + a.stage](a)


if __name__ == "__main__":
    main()
