#!/usr/bin/env python3
"""Issue #18 (m1-04): interleaved A/B of the deterministic Leiden sub-round rule.

Runs lab/b2-leiden's `b2` (built from this branch) round-robin: for every round, the blocks
(graph, threads, iterations) are visited in a seeded shuffled order and, inside a block, every
variant (the deterministic sub-round settings and the asynchronous mode) runs once, also in a
seeded shuffled order. Ratios are taken inside a block (same round, same graph, threads and
iterations, minutes apart at most), which is what makes them robust to background load.

Every run is its own process under
    systemd-run --user --scope -p MemoryMax=<limit> -p MemorySwapMax=0
and appends one JSON line (b2's output plus round/block/position and the load average just
before it) to --out. Rows already present in --out are skipped, so an interrupted run resumes.

Usage:
    python3 ab.py --out results/ab-lj-orkut.jsonl                  # the full matrix (defaults)
    python3 ab.py --graphs friendster --threads 16 --iterations 1 --mem 24G --rounds 1 \
        --variants auto,s16,s64,L16k,async --out results/ab-friendster.jsonl
"""
import argparse
import json
import os
import random
import resource
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
B2 = os.path.join(HERE, "..", "b2-leiden", "target", "release", "b2")
DATA = os.environ.get("EVERYGRAPH_DATA", "/home/tux/graphlab/data")
# Friendster does not fit b2 in RAM under 24 GiB (23.6 GB anonymous + 15 GB mapped graph with the
# f32 build: lab/b2-leiden/results/big). It runs through lab/m1-02-ooc-leiden's m102: the same local
# moving and sub-round code as b2, compressed level 0 mapped lazily, sequential aggregation, coarse
# levels spilled to SPILL (emptied before and after every run). m102 got the same --sub-div/--sub-min/
# --sub-max knobs as b2 in this branch (defaults unchanged).
M102 = os.path.join(HERE, "..", "m1-02-ooc-leiden", "target", "release", "m102")
SPILL = os.environ.get("M104_SPILL", "/home/tux/graphlab/scratch/m1-04-det-leiden-scaling/spill")
OOC_GRAPHS = {"friendster"}


def clear_spill():
    if os.path.isdir(SPILL):
        shutil.rmtree(SPILL)

# name -> extra arguments. Every deterministic variant's sub-round count is a function of the
# current level's vertex count only (never of threads or budget).
VARIANTS = {
    "auto": ["--det"],  # today: clamp(next_pow2(n/16384), 256, 1024) at every level
    "s4": ["--det", "--sub-rounds", "4"],
    "s8": ["--det", "--sub-rounds", "8"],
    "s16": ["--det", "--sub-rounds", "16"],
    "s32": ["--det", "--sub-rounds", "32"],
    "s64": ["--det", "--sub-rounds", "64"],
    # level-size rules: ~16k (resp. ~64k) vertices per sub-round, never fewer than 4 sub-rounds
    "L16k": ["--det", "--sub-div", "16384", "--sub-min", "4", "--sub-max", "1024"],
    "L64k": ["--det", "--sub-div", "65536", "--sub-min", "4", "--sub-max", "1024"],
    # the issue's literal candidate: 16 on large levels, down to 4 on coarse ones
    "L64k-max16": ["--det", "--sub-div", "65536", "--sub-min", "4", "--sub-max", "16"],
    "async": [],
}


def machine_busy_s():
    """User + system CPU seconds of the whole machine (all CPUs), from /proc/stat."""
    with open("/proc/stat") as f:
        v = f.readline().split()[1:]
    hz = os.sysconf("SC_CLK_TCK")
    return (int(v[0]) + int(v[1]) + int(v[2])) / hz  # user, nice, system


def child_cpu_s():
    ru = resource.getrusage(resource.RUSAGE_CHILDREN)
    return ru.ru_utime + ru.ru_stime


def loadavg():
    with open("/proc/loadavg") as f:
        return float(f.read().split()[0])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--graphs", nargs="+", default=["lj", "orkut"])
    ap.add_argument("--threads", nargs="+", type=int, default=[1, 4, 16])
    ap.add_argument("--iterations", nargs="+", type=int, default=[1, 3])
    ap.add_argument("--variants", default="auto,s4,s8,s16,s32,s64,L16k,L64k,L64k-max16,async")
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--seed", type=int, default=18)
    ap.add_argument("--mem", default="8G")
    ap.add_argument("--timeout", type=int, default=900)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    assert max(a.threads) <= 16, "at most 16 threads on the shared machine"
    variants = a.variants.split(",")
    for v in variants:
        assert v in VARIANTS, v
    done = set()
    if os.path.exists(a.out):
        with open(a.out) as f:
            for line in f:
                r = json.loads(line)
                done.add((r["round"], r["graph"], r["threads"], r["iterations"], r["variant"]))
    rng = random.Random(a.seed)
    blocks = [(g, t, it) for g in a.graphs for t in a.threads for it in a.iterations]
    for rnd in range(a.rounds):
        order = blocks[:]
        rng.shuffle(order)
        for bi, (g, t, it) in enumerate(order):
            vs = variants[:]
            rng.shuffle(vs)
            for pos, v in enumerate(vs):
                if (rnd, g, t, it, v) in done:
                    continue
                wrap = ["systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={a.mem}", "-p", "MemorySwapMax=0"]
                if g in OOC_GRAPHS:
                    clear_spill()
                    cmd = [*wrap, M102, "leiden", "run", "--store", f"{DATA}/store-m113/{g}", "--config", "comp",
                           "--algo", "leiden", "--threads", str(t), "--iterations", str(it), "--mem", "mmap-lazy",
                           "--ooc", "--spill", SPILL, *VARIANTS[v]]
                else:
                    cmd = [*wrap, B2, "run", "--graph", f"{DATA}/csr/{g}", "--algo", "leiden", "--threads", str(t),
                           "--iterations", str(it), *VARIANTS[v]]
                la = loadavg()
                m0, c0 = machine_busy_s(), child_cpu_s()
                t0 = time.time()
                try:
                    p = subprocess.run(cmd, capture_output=True, text=True, timeout=a.timeout)
                    status = "ok" if p.returncode == 0 else f"exit {p.returncode}"
                    out = p.stdout.strip().splitlines()
                    row = json.loads(out[-1]) if p.returncode == 0 and out else {"stderr": p.stderr[-2000:]}
                except subprocess.TimeoutExpired:
                    status, row = "timeout", {}
                wall = time.time() - t0
                if g in OOC_GRAPHS:
                    clear_spill()
                own = child_cpu_s() - c0
                others = max(0.0, (machine_busy_s() - m0 - own) / wall)
                row.update(
                    own_cpu_s=round(own, 2), others_busy_cores=round(others, 2),
                    round=rnd, block=bi, position=pos, variant=v, graph=g, threads=t, iterations=it,
                    status=status, loadavg_before=la, wall_s=round(wall, 3), mem_limit=a.mem,
                    args=VARIANTS[v],
                )
                with open(a.out, "a") as f:
                    f.write(json.dumps(row) + "\n")
                print(f"r{rnd} {g} t{t} it{it} {v:>10}: {status} {row.get('time_s')} s Q {row.get('modularity')} "
                      f"load {la} others {others:.1f} cores", flush=True)


if __name__ == "__main__":
    sys.exit(main())
