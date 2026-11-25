#!/usr/bin/env bash
# Exploratory runs (development only; numbers from this script are never quoted: the machine is
# shared while it runs). Official runs go through bench/specs/ooc-leiden.toml.
#
#   ./dev.sh LIMIT [m102 leiden run flags...]      e.g. ./dev.sh 447M --store S/orkut --mem mmap-lazy --ooc --spill S/spill
#
# Runs one Leiden under a hard memory limit, after evicting the store named by --store, and
# prints a one-line summary. The full JSON line is appended to $OUT (default $S/dev.jsonl).
set -u
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-02-ooc-leiden}
OUT=${OUT:-$S/dev.jsonl}
B=./target/release/m102
LIM=$1
shift
store=""
prev=""
for a in "$@"; do
  [ "$prev" = "--store" ] && store=$a
  prev=$a
done
[ -n "$store" ] && [ -z "${WARM:-}" ] && $B evict --store "$store"
systemd-run --user --scope -q -p MemoryMax="$LIM" -p MemorySwapMax=0 \
  $B leiden run --config comp --threads "${T:-8}" "$@" > "$S/last.json" 2> "$S/last.err"
ec=$?
tail -n 1 "$S/last.json" >> "$OUT"
python3 -I - "$S/last.json" "$S/last.err" "$LIM" "$ec" <<'PY'
import json, sys
js, err, lim, ec = sys.argv[1:]
try:
    d = json.loads(open(js).read().strip().splitlines()[-1])
except Exception:
    print(lim, "exit", ec, open(err).read()[-400:]); sys.exit()
if d.get("status") != "ok":
    print(lim, "exit", ec, d); sys.exit()
keys = ["time_s", "level0_lm_s", "level0_refine_s", "level0_agg_s", "upper_levels_s", "eval_s", "modularity", "communities", "disconnected", "hash_levels"]
s = {k: d.get(k) for k in keys}
s["anon_peak_MiB"] = d["rss_anon_peak_kb"] // 1024
s["acct_peak_MiB"] = d["accountant"]["peak_bytes"] // 2**20
s["budget_MiB"] = (d["budget_bytes"] or 0) // 2**20
s["cg_peak_MiB"] = d["cgroup"]["memory_peak"] // 2**20
s["majflt"] = d["cgroup"]["pgmajfault"]
s["read_GB"] = round(d.get("read_bytes", 0) / 1e9, 2)
s["write_GB"] = round(d.get("write_bytes", 0) / 1e9, 2)
a = d["passes"][0]["seq_agg"]
if a:
    s["agg0"] = {k: a[k] for k in ["emit_s", "process_s", "bucket_ram_bytes", "bucket_disk_bytes", "records", "mem_waits"]}
print(lim, "exit", ec, s)
PY
