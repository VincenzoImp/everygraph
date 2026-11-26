#!/usr/bin/env python3
"""measure.py --out RUN.json --limit 8G [--timeout S] [--tag K=V ...] -- CMD ...

Run CMD in its own process group, poll the anonymous RSS of its whole process tree every
0.25 s, and write one JSON record: wall, user and sys seconds, peak RSS (getrusage, max over
children) and peak anonymous RSS (sum of RssAnon over the tree, sampled). The memory limit
itself is set by the caller's `systemd-run --scope`; --limit only records it.
On timeout the recorded process group is killed and rc is -9.
"""
import argparse, json, os, resource, signal, subprocess, sys, time

ap = argparse.ArgumentParser()
ap.add_argument("--out", required=True)
ap.add_argument("--limit", required=True)
ap.add_argument("--timeout", type=float, default=0)
ap.add_argument("--tag", action="append", default=[])
ap.add_argument("cmd", nargs=argparse.REMAINDER)
a = ap.parse_args()
cmd = a.cmd[1:] if a.cmd and a.cmd[0] == "--" else a.cmd


def tree(pid):
    out, todo = [], [pid]
    while todo:
        p = todo.pop()
        out.append(p)
        try:
            for t in os.listdir(f"/proc/{p}/task"):
                with open(f"/proc/{p}/task/{t}/children") as f:
                    todo += [int(c) for c in f.read().split()]
        except OSError:
            pass
    return out


def anon_kb(pid):
    s = 0
    for p in tree(pid):
        try:
            with open(f"/proc/{p}/status") as f:
                for line in f:
                    if line.startswith("RssAnon:"):
                        s += int(line.split()[1])
        except OSError:
            pass
    return s


load0 = os.getloadavg()
t0 = time.time()
child = subprocess.Popen(cmd, start_new_session=True)
peak, timed_out = 0, False
while child.poll() is None:
    peak = max(peak, anon_kb(child.pid))
    if a.timeout and time.time() - t0 > a.timeout:
        timed_out = True
        os.killpg(child.pid, signal.SIGKILL)
        child.wait()
        break
    time.sleep(0.25)
wall = time.time() - t0
r = resource.getrusage(resource.RUSAGE_CHILDREN)
rec = {
    "cmd": cmd, "rc": child.returncode, "timed_out": timed_out,
    "wall_s": round(wall, 2), "user_s": round(r.ru_utime, 1), "sys_s": round(r.ru_stime, 1),
    "peak_rss_mb": round(r.ru_maxrss / 1024), "peak_anon_mb_sampled": round(peak / 1024),
    "memory_limit": a.limit, "swap": "MemorySwapMax=0",
    "loadavg_start": [round(x, 2) for x in load0], "loadavg_end": [round(x, 2) for x in os.getloadavg()],
    "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(t0)),
    "timings": "exploratory (shared machine, other agents compiling)",
}
for kv in a.tag:
    k, v = kv.split("=", 1)
    rec[k] = v
json.dump(rec, open(a.out, "w"), indent=1)
print(json.dumps(rec), file=sys.stderr)
sys.exit(0 if child.returncode == 0 else 1)
