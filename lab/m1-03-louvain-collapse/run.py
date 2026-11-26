#!/usr/bin/env python3
"""m1-03 (#17) runner: one b2 run per line of a plan, results appended as JSON lines.

Each row keeps b2's own JSON plus the exact argv, the memory limit, the build (w32 = f32
quotient weights, f64), the load average before the run and the wall time of the process.

    python3 run.py --plan full16 --out results/full.jsonl      # Friendster, FINDINGS/tracer flags
    python3 run.py --plan replay --graph-dir L1 --out results/replay.jsonl

Builds: B2_F64 / B2_W32 environment variables, default the scratch targets of this issue.
"""
import argparse
import json
import os
import random
import subprocess
import sys
import time

DATA = os.environ.get("EVERYGRAPH_DATA", "/home/tux/graphlab/data")
SCRATCH = "/home/tux/graphlab/scratch/m1-03-louvain-collapse"
B2 = {
    "f64": os.environ.get("B2_F64", f"{SCRATCH}/target/release/b2"),
    "w32": os.environ.get("B2_W32", f"{SCRATCH}/target-w32/release/b2"),
}
M102 = os.environ.get("M102", f"{SCRATCH}/target-m102/release/m102")
# lab/b2-leiden/run_friendster.sh (the FINDINGS § B2 collapse row) and lab/m1-01-tracer/tracer.py
# (B2_FLAGS + HIER_FLAGS["plain"] + --out-hier). Both used 32 threads; this machine is shared
# tonight, so runs here use at most 16.
FINDINGS_FLAGS = ["--lowmem", "--no-populate", "--verbose"]
TRACER_FLAGS = ["--lowmem", "--no-populate", "--min-ratio", "3", "--verbose"]


def run(build, graph, threads, flags, mem, timeout, tag, out, extra_meta=None, algo="louvain"):
    work = f"{SCRATCH}/work"
    os.makedirs(work, exist_ok=True)
    jpath = f"{work}/{tag}.json"
    if os.path.exists(jpath):
        os.remove(jpath)
    argv = [B2[build], "run", "--graph", graph, "--algo", algo, "--threads", str(threads)] + flags + ["--json", jpath]
    cmd = ["systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={mem}", "-p", "MemorySwapMax=0", "timeout", str(timeout)] + argv
    load = os.getloadavg()[0]
    t = time.time()
    with open(f"{work}/{tag}.log", "w") as log:
        rc = subprocess.call(cmd, stdout=subprocess.DEVNULL, stderr=log)
    wall = time.time() - t
    row = {"tag": tag, "build": build, "mem": mem, "argv": argv[1:], "rc": rc, "wall_s": round(wall, 2), "load_before": load}
    row.update(extra_meta or {})
    if rc == 0 and os.path.exists(jpath):
        row["b2"] = json.load(open(jpath))
    with open(out, "a") as f:
        f.write(json.dumps(row) + "\n")
    b = row.get("b2", {})
    print(f"{tag}: rc {rc} Q {b.get('modularity')} comms {b.get('communities')} largest {b.get('largest_nodes')} "
          f"iters {[p['iters'] for p in b.get('passes', [])]} wall {wall:.0f}s load {load:.1f}", flush=True)
    return row


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--plan", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--graph-dir")
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--threads", type=int, nargs="+", default=[16])
    ap.add_argument("--mems", nargs="+", default=["16G", "24G"])
    ap.add_argument("--seeds", type=int, default=8)
    ap.add_argument("--flags", default="", help="extra b2 flags for replay plans")
    ap.add_argument("--builds", nargs="+", default=["w32", "f64"])
    a = ap.parse_args()
    fr = f"{DATA}/csr/friendster"
    if a.plan == "full":
        # FINDINGS and tracer flags interleaved, under each memory limit, w32 build as both originals.
        jobs = []
        for r in range(a.reps):
            for mem in a.mems:
                for name, fl in (("findings", FINDINGS_FLAGS), ("tracer", TRACER_FLAGS)):
                    for t in a.threads:
                        jobs.append((name, fl, mem, t, r))
        lab = a.flags.replace("--", "").replace(" ", "")
        for name, fl, mem, t, r in jobs:
            run("w32", fr, t, fl + ["--trace"] + a.flags.split(), mem, 900, f"full-{name}{lab}-{mem}-t{t}-r{r}", a.out,
                {"config": name, "rep": r, "threads": t, "flags": a.flags})
    elif a.plan == "dump":
        # pass 0 on Friendster, f64 build (exact quotient weights), level-1 graph written for replays
        for t in a.threads:
            d = f"{SCRATCH}/L1-t{t}"
            run("f64", fr, t, FINDINGS_FLAGS + ["--trace", "--dump-level1", d], a.mems[0], 1800, f"dump-t{t}", a.out,
                {"config": "dump", "dump_dir": d})
    elif a.plan == "replay":
        # passes 1.. of one level-0 result: tol 0.001 (pass 1 of the original schedule), 9 passes left
        g = a.graph_dir
        base = ["--tol", "0.001", "--max-passes", "9", "--trace", "--trace-q0", "--node-weights", f"{g}/nodes.u32"] + a.flags.split()
        jobs = [(b, t, s) for b in a.builds for t in a.threads for s in range(a.seeds)]
        random.Random(17).shuffle(jobs)
        for b, t, s in jobs:
            run(b, g, t, base + ["--order-seed", str(s)], "8G", 600,
                f"replay-{os.path.basename(g)}-{b}-t{t}-s{s}", a.out,
                {"config": "replay", "level1": g, "seed": s, "threads": t, "flags": a.flags})
    elif a.plan == "small":
        # lj / orkut: async Louvain and Leiden over order seeds and threads
        for gname in ("lj", "orkut"):
            for algo in ("louvain", "leiden"):
                jobs = [(t, s) for t in a.threads for s in range(a.seeds)]
                random.Random(17).shuffle(jobs)
                for t, s in jobs:
                    fl = ["--lowmem", "--trace", "--order-seed", str(s)] + a.flags.split()
                    run(a.builds[0], f"{DATA}/csr/{gname}", t, fl, "8G", 600,
                        f"small-{gname}-{algo}-t{t}-s{s}", a.out,
                        {"config": "small", "graph": gname, "algo": algo, "seed": s, "threads": t, "flags": a.flags}, algo)
    elif a.plan == "leiden-ooc":
        # async Leiden on Friendster through lab/m1-02-ooc-leiden's m102 (the m1-04 command; b2 needs
        # 23.6 GB anonymous in RAM, over this run's 24G limit), with and without --heavy
        spill = f"{SCRATCH}/spill"
        variants = [("base", []), ("heavy", ["--heavy", "0.001"])]
        jobs = [(v, fl, r) for r in range(a.reps) for v, fl in variants]
        for v, fl, r in jobs:
            subprocess.call(["rm", "-rf", spill])
            os.makedirs(spill)
            argv = [M102, "leiden", "run", "--store", f"{DATA}/store-m113/friendster", "--config", "comp", "--algo", "leiden",
                    "--threads", str(a.threads[0]), "--iterations", "1", "--mem", "mmap-lazy", "--ooc", "--spill", spill] + fl
            cmd = ["systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={a.mems[0]}", "-p", "MemorySwapMax=0", "timeout", "1800"] + argv
            load = os.getloadavg()[0]
            t = time.time()
            p = subprocess.run(cmd, capture_output=True, text=True)
            wall = time.time() - t
            subprocess.call(["rm", "-rf", spill])
            row = {"tag": f"leiden-ooc-{v}-r{r}", "config": "leiden-ooc", "variant": v, "mem": a.mems[0], "argv": argv,
                   "rc": p.returncode, "wall_s": round(wall, 2), "load_before": load}
            lines = p.stdout.strip().splitlines()
            if p.returncode == 0 and lines:
                row["b2"] = json.loads(lines[-1])
            else:
                row["stderr"] = p.stderr[-2000:]
            with open(a.out, "a") as f:
                f.write(json.dumps(row) + "\n")
            b = row.get("b2", {})
            print(f"{row['tag']}: rc {p.returncode} Q {b.get('modularity')} comms {b.get('communities')} "
                  f"iters {[q['iters'] for q in b.get('passes', [])]} wall {wall:.0f}s load {load:.1f}", flush=True)
    else:
        sys.exit(f"unknown plan {a.plan}")


if __name__ == "__main__":
    main()
