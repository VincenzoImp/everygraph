#!/usr/bin/env python3
"""Number of CPUs busy right now (all processes), sampled from /proc/stat over 0.5 s.

bench.sh calls this before every measurement, while nothing of its own is running, so the value
is the load caused by *other* work. The 1-minute load average cannot be used as a gate between
runs because our own previous run keeps it high for a minute; it is still recorded in every row.
"""
import time


def snap():
    with open("/proc/stat") as f:
        v = [int(x) for x in f.readline().split()[1:]]
    idle = v[3] + v[4]
    return sum(v[:8]), idle


t0, i0 = snap()
time.sleep(0.5)
t1, i1 = snap()
ncpu = sum(1 for l in open("/proc/stat") if l.startswith("cpu") and l[3].isdigit())
dt = max(t1 - t0, 1)
print(f"{ncpu * (1 - (i1 - i0) / dt):.2f}")
