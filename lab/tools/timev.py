#!/usr/bin/env python3
"""timev.py CMD... — run a command; print wall seconds and peak RSS (max over direct children) as JSON on stderr."""
import json, resource, subprocess, sys, time
t = time.time(); rc = subprocess.call(sys.argv[1:]); r = resource.getrusage(resource.RUSAGE_CHILDREN)
print(json.dumps({"cmd": " ".join(sys.argv[1:])[:200], "rc": rc, "wall_s": round(time.time() - t, 2),
                  "user_s": round(r.ru_utime, 1), "sys_s": round(r.ru_stime, 1), "peak_rss_mb": round(r.ru_maxrss / 1024)}), file=sys.stderr)
sys.exit(rc)
