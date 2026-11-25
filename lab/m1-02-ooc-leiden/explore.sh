#!/usr/bin/env bash
# Exploratory records for the claims of PR #143 that no official row carries (review M2).
# Exploratory: one process at a time on an otherwise idle machine, not through the harness;
# the files say what ran, they are not timings to quote.
#
#   results/determinism.jsonl   deterministic Leiden on lj and orkut: in RAM vs out of core at
#                               small budgets (buckets on disk, split communities), --check-agg
#                               at every level, lj also at 1 and 4 threads
#   results/map_identity.jsonl  map-shaped deterministic orkut, in RAM vs out of core, plus one
#                               line comparing the level files byte for byte
#   results/orders.jsonl        survey ideas L1-L3 on lj and orkut (orders.sh)
#   results/friendster_idorder.jsonl  Friendster in id order on the original ids, under 8 GiB
#   results/default_agg_oom.jsonl     Friendster in RAM with the default (holey) aggregation
#                                     under 48 GiB: the exit status
set -u
cd "$(dirname "$0")"
D=/home/tux/graphlab/data/store-m113
S=${S:-/home/tux/graphlab/scratch/m1-02-ooc-leiden}
export S
mkdir -p results "$S"
R=$PWD/results
B=./target/release/m102
export T=16

# --- determinism and --check-agg
export OUT=$R/determinism.jsonl
: > "$OUT"
for g in lj orkut; do
  ./dev.sh 8G --store $D/$g --name $g --mem mmap --det
  for bud in 0.25 0.3 0.6; do
    ./dev.sh 4G --store $D/$g --name $g --mem mmap-lazy --det --ooc --budget-gb $bud --spill "$S/spill" --check-agg
  done
done
for t in 1 4; do
  T=$t ./dev.sh 4G --store $D/lj --name lj --mem mmap-lazy --det --ooc --budget-gb 0.3 --spill "$S/spill"
done

# --- map-shaped identity
export OUT=$R/map_identity.jsonl
: > "$OUT"
rm -rf "$S/hier_ram" "$S/hier_ooc"
./dev.sh 8G --store $D/orkut --name orkut --mem mmap --det --cap 64 --max-passes 20 --min-ratio 3 --out-hier "$S/hier_ram"
./dev.sh 4G --store $D/orkut --name orkut --mem mmap-lazy --det --ooc --spill "$S/spill" --cap 64 --max-passes 20 --min-ratio 3 --out-hier "$S/hier_ooc"
python3 -I - "$S/hier_ram" "$S/hier_ooc" >> "$OUT" <<'PY'
import filecmp, json, os, sys
a, b = sys.argv[1:]
files = sorted(f for f in os.listdir(a) if f.startswith("level_"))
same = {f: filecmp.cmp(os.path.join(a, f), os.path.join(b, f), shallow=False) for f in files}
print(json.dumps({"check": "map-shaped orkut, deterministic: level files in RAM vs out of core", "files": len(files), "identical": all(same.values()), "per_file": same}))
PY
rm -rf "$S/hier_ram" "$S/hier_ooc"

# --- survey ideas L1-L3 on lj and orkut
OUT=$R/orders.jsonl
export LIMIT_lj=400M LIMIT_orkut=447M
: > "$OUT"
OUT=$OUT ./orders.sh

# --- Friendster in id order on the original ids
export OUT=$R/friendster_idorder.jsonl
: > "$OUT"
./dev.sh 8G --store $D/friendster --name friendster --mem mmap-lazy --ooc --spill "$S/spill" --natural

# --- default (holey) aggregation in RAM under 48 GiB
out=$R/default_agg_oom.jsonl
$B evict --store $D/friendster
systemd-run --user --scope -q -p MemoryMax=48G -p MemorySwapMax=0 \
  $B leiden run --config comp --threads 16 --store $D/friendster --mem mmap > "$S/holey.json" 2> "$S/holey.err"
ec=$?
python3 -I - "$ec" "$S/holey.json" "$S/holey.err" > "$out" <<'PY'
import json, sys
ec, js, err = sys.argv[1:]
lines = [l for l in open(js) if l.startswith("{")]
print(json.dumps({"case": "friendster in RAM, default (holey) aggregation, --mem mmap (pre-faulted), 16 threads, MemoryMax=48G MemorySwapMax=0",
                  "exit_status": int(ec), "killed_by_oom": int(ec) == 137, "completed": bool(lines),
                  "stderr_tail": open(err).read()[-600:]}))
PY
rm -rf "$S/spill"
