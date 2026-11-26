#!/usr/bin/env python3
"""Issue #172 (m1-15): run one command inside the current cgroup scope and report what the
cgroup charged while it ran.

    cgprobe.py [--evict PATH]... [--hash FILE]... [--interval S] -- COMMAND...

Runs inside the `systemd-run --user --scope -p MemoryMax=... -p MemorySwapMax=0` scope that
the benchmark harness (or run_ab.py) creates, so memory.stat is the command's alone.

* `--evict`: drop the clean page-cache pages of these files (directories: every file under
  them) with posix_fadvise(DONTNEED) before the command starts. The harness evicts the
  dataset's private copy; this evicts side inputs (the hierarchy).
* Samples memory.stat every `--interval` seconds (default 0.1) while the command runs and
  keeps the peak of the kernel-memory terms the issue asks for (`pagetables`, `kernel`,
  `shmem`, and the other kernel terms) and of `anon` and `file`.
* After the command exits (and is reaped), reads the cumulative counters of memory.stat
  (`workingset_refault_file`, `pgmajfault`, ...), memory.peak, memory.events, and
  /proc/self/io: `read_bytes` there includes every reaped descendant, so the difference from
  the value before the command started is the command's own storage reads, page-fault
  reads of mapped files included (the cgroup's io.stat is not available: systemd does
  not delegate the io controller to user scopes on this machine, bench/README.md).
* `--hash`: SHA-256 of these output files, computed after the I/O counters were read.

Stdout of the command is passed through; the last line printed is one JSON object
{"tool": <the command's last JSON line or null>, "cg": {...}, "exit": code}, which the
harness reads metrics from. Exits with the command's status.
"""
import hashlib
import json
import os
import subprocess
import sys
import threading
import time

PEAK_KEYS = ["anon", "file", "kernel", "kernel_stack", "pagetables", "sec_pagetables", "percpu",
             "sock", "vmalloc", "shmem", "file_mapped", "slab", "slab_reclaimable", "slab_unreclaimable"]
COUNTERS = ["workingset_refault_file", "workingset_refault_anon", "workingset_activate_file",
            "workingset_restore_file", "workingset_nodereclaim", "pgfault", "pgmajfault",
            "pgscan", "pgsteal", "pgscan_kswapd", "pgscan_direct", "pgrefill", "pgactivate",
            "pgdeactivate"]


def cgroup_dir():
    for line in open("/proc/self/cgroup"):
        if line.startswith("0::"):
            return "/sys/fs/cgroup" + line[3:].strip()
    raise SystemExit("cgprobe: no cgroup v2 entry in /proc/self/cgroup")


def read_kv(path):
    out = {}
    try:
        for line in open(path):
            k, v = line.split()
            out[k] = int(v)
    except (FileNotFoundError, ValueError):
        pass
    return out


def read_io():
    out = {}
    for line in open("/proc/self/io"):
        k, v = line.split(":")
        out[k] = int(v)
    return out


def files_under(paths):
    for p in paths:
        if os.path.isdir(p):
            for root, _, fs in os.walk(p):
                for f in sorted(fs):
                    yield os.path.join(root, f)
        elif os.path.isfile(p):
            yield p


def evict(paths):
    n = 0
    for f in files_under(paths):
        fd = os.open(f, os.O_RDONLY)
        try:
            os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
            n += os.fstat(fd).st_size
        finally:
            os.close(fd)
    return n


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            b = f.read(1 << 22)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def main():
    argv = sys.argv[1:]
    if "--" not in argv:
        raise SystemExit(__doc__)
    i = argv.index("--")
    opts, cmd = argv[:i], argv[i + 1:]
    ev, hs, interval = [], [], 0.1
    j = 0
    while j < len(opts):
        if opts[j] == "--evict":
            ev.append(opts[j + 1])
        elif opts[j] == "--hash":
            hs.append(opts[j + 1])
        elif opts[j] == "--interval":
            interval = float(opts[j + 1])
        else:
            raise SystemExit(f"cgprobe: unknown option {opts[j]}")
        j += 2
    cg = cgroup_dir()
    evicted = evict(ev)
    stat0 = read_kv(f"{cg}/memory.stat")
    io0 = read_io()
    peaks = {k: 0 for k in PEAK_KEYS}
    samples = [0]
    stop = threading.Event()

    def sampler():
        while not stop.is_set():
            s = read_kv(f"{cg}/memory.stat")
            for k in PEAK_KEYS:
                peaks[k] = max(peaks[k], s.get(k, 0))
            samples[0] += 1
            stop.wait(interval)

    th = threading.Thread(target=sampler, daemon=True)
    th.start()
    t0 = time.monotonic()
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, text=True)
    last_json = None
    for line in p.stdout:
        sys.stdout.write(line)
        s = line.strip()
        if s.startswith("{"):
            try:
                last_json = json.loads(s)
            except ValueError:
                pass
    rc = p.wait()
    wall = time.monotonic() - t0
    stop.set()
    th.join()
    io1 = read_io()
    stat1 = read_kv(f"{cg}/memory.stat")
    for k in PEAK_KEYS:
        peaks[k] = max(peaks[k], stat1.get(k, 0))
    try:
        mpeak = int(open(f"{cg}/memory.peak").read())
    except (FileNotFoundError, ValueError):
        mpeak = None
    res = {
        "cgroup": cg,
        "wall_seconds": wall,
        "read_bytes": io1["read_bytes"] - io0["read_bytes"],
        "write_bytes": io1["write_bytes"] - io0["write_bytes"],
        "rchar": io1["rchar"] - io0["rchar"],
        "evicted_side_input_bytes": evicted,
        "memory_peak_bytes": mpeak,
        "memory_max": open(f"{cg}/memory.max").read().strip(),
        "memory_events": read_kv(f"{cg}/memory.events"),
        "peak": peaks,
        "samples": samples[0],
        "interval_s": interval,
        "delta": {k: stat1.get(k, 0) - stat0.get(k, 0) for k in COUNTERS},
    }
    res["hashes"] = {os.path.basename(f): sha256(f) for f in hs if os.path.isfile(f)}
    sys.stdout.write(json.dumps({"tool": last_json, "cg": res, "exit": rc}) + "\n")
    sys.stdout.flush()
    sys.exit(rc if rc >= 0 else 128 - rc)


if __name__ == "__main__":
    main()
