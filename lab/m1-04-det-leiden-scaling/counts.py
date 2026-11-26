#!/usr/bin/env python3
"""Load-independent structure of each variant: sub-rounds run by all threads (3 barriers each) or by
one thread, per level, plus work (edge passes) and iterations. One run per (graph, iterations,
variant) with the instrumented b2 (the counters were added after the A/B had started; that binary
was built in target-m104 so as not to replace the running one, and is byte-identical to the final
target/release build). Its hash must equal the A/B's: the counters change no result. Timings
here are not used.

    python3 counts.py --out results/counts-lj-orkut.jsonl
    python3 counts.py --variants s16,s64 --extra "--seq-below 4096" --tag seq4096 --out ...
"""
import argparse
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ab import DATA, HERE, VARIANTS  # noqa: E402

B2 = os.path.join(HERE, "..", "b2-leiden", "target", "release", "b2")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--graphs", nargs="+", default=["lj", "orkut"])
    ap.add_argument("--iterations", nargs="+", type=int, default=[1, 3])
    ap.add_argument("--variants", default="auto,s4,s8,s16,s32,s64,L16k,L64k,L64k-max16")
    ap.add_argument("--extra", default="", help="extra b2 arguments, e.g. '--seq-below 4096'")
    ap.add_argument("--tag", default="")
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--mem", default="8G")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    for g in a.graphs:
        for it in a.iterations:
            for v in a.variants.split(","):
                cmd = ["systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={a.mem}", "-p", "MemorySwapMax=0",
                       B2, "run", "--graph", f"{DATA}/csr/{g}", "--algo", "leiden", "--threads", str(a.threads),
                       "--iterations", str(it), *VARIANTS[v], *a.extra.split()]
                p = subprocess.run(cmd, capture_output=True, text=True, check=True)
                r = json.loads(p.stdout.strip().splitlines()[-1])
                r.update(variant=v, graph=g, iterations=it, extra=a.extra, tag=a.tag)
                with open(a.out, "a") as f:
                    f.write(json.dumps(r) + "\n")
                print(g, it, v, a.extra, r["hash_levels"], r["subrounds_par"], r["subrounds_seq"], r["time_s"], flush=True)


if __name__ == "__main__":
    sys.exit(main())
